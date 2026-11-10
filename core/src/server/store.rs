//! 内存频道存储（对应 Go `store.go` 的 `MemoryStore`）：频道分配、TTL 过期、
//! blob 存档、CPace 失败计数与每 IP 前缀频道上限。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::ratelimit::ip_prefix;

/// 单频道最大 blob 存档数（对应 Go `DefaultMaxBlobsPerChannel`）。
pub const DEFAULT_MAX_BLOBS_PER_CHANNEL: usize = 10;
/// 频道 CPace 失败上限（对应 Go `DefaultMaxCPaceFailures`）。
pub const DEFAULT_MAX_CPACE_FAILURES: usize = 3;
/// 每 IP 前缀的活跃频道上限（对应 Go `DefaultMaxChannelsPerIP`）。
pub const DEFAULT_MAX_CHANNELS_PER_IP: usize = 100;

/// 内存频道存储。
pub struct MemoryStore {
    inner: Mutex<Inner>,
    max_channels_per_ip: usize,
}

struct Inner {
    channels: HashMap<u16, MemChannel>,
    /// IP 前缀 → 活跃频道数。
    channel_owners: HashMap<String, usize>,
}

#[derive(Default)]
struct MemChannel {
    expires: Option<Instant>,
    failures: usize,
    /// [0] = 接收端存的，[1] = 发送端存的。
    blobs: [Vec<u8>; 2],
    owner: Option<String>,
}

impl MemoryStore {
    /// 新建；`max_channels_per_ip` 为 0 表示不限制。
    pub fn new(max_channels_per_ip: usize) -> Self {
        Self {
            inner: Mutex::new(Inner {
                channels: HashMap::new(),
                channel_owners: HashMap::new(),
            }),
            max_channels_per_ip,
        }
    }

    /// 注册新频道；重复 ID 或超每 IP 上限都报错。
    pub fn allocate_channel(
        &self,
        id: u16,
        ttl: Duration,
        remote: std::net::SocketAddr,
    ) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if inner.channels.contains_key(&id) {
            return Err(format!("channel {id} already exists"));
        }
        if self.max_channels_per_ip > 0 {
            let prefix = ip_prefix(remote);
            let count = inner.channel_owners.entry(prefix.clone()).or_default();
            if *count >= self.max_channels_per_ip {
                return Err(format!(
                    "channel limit reached for {prefix} ({} active)",
                    count
                ));
            }
            *count += 1;
            inner.channels.insert(
                id,
                MemChannel {
                    expires: Some(Instant::now() + ttl),
                    owner: Some(prefix),
                    ..MemChannel::default()
                },
            );
        } else {
            inner.channels.insert(
                id,
                MemChannel {
                    expires: Some(Instant::now() + ttl),
                    ..MemChannel::default()
                },
            );
        }
        Ok(())
    }

    /// 频道已分配且未过期。
    pub fn channel_exists(&self, id: u16) -> bool {
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner
            .channels
            .get(&id)
            .is_some_and(|ch| ch.expires.is_some_and(|t| Instant::now() < t))
    }

    /// 存一份加密 blob（sender=true 表示来自发送端）。
    pub fn store_blob(&self, id: u16, sender: bool, blob: &[u8]) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let channel = inner
            .channels
            .get_mut(&id)
            .ok_or_else(|| format!("channel {id} not found"))?;
        channel
            .expires
            .is_some_and(|t| Instant::now() <= t)
            .then_some(())
            .ok_or_else(|| format!("channel {id} expired"))?;
        channel.blobs[usize::from(sender)] = blob.to_vec();
        Ok(())
    }

    /// 取指定侧存过的 blob（没有则 None；频道不存在也返回 None，与 Go 一致）。
    pub fn fetch_blob(&self, id: u16, sender: bool) -> Result<Option<Vec<u8>>, String> {
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let Some(channel) = inner.channels.get(&id) else {
            return Ok(None);
        };
        if !channel.expires.is_some_and(|t| Instant::now() <= t) {
            return Err(format!("channel {id} expired"));
        }
        Ok(Some(channel.blobs[usize::from(sender)].clone()))
    }

    /// 递增失败计数，返回新值。
    pub fn record_failure(&self, id: u16) -> Result<usize, String> {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let channel = inner
            .channels
            .get_mut(&id)
            .ok_or_else(|| format!("channel {id} not found"))?;
        channel.failures += 1;
        Ok(channel.failures)
    }

    /// 删除频道（含归属计数回退）。
    pub fn delete_channel(&self, id: u16) {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        Self::delete_locked(&mut inner, id);
    }

    fn delete_locked(inner: &mut Inner, id: u16) {
        if let Some(channel) = inner.channels.remove(&id)
            && let Some(owner) = channel.owner
        {
            let count = inner.channel_owners.entry(owner.clone()).or_default();
            *count = count.saturating_sub(1);
            if *count == 0 {
                inner.channel_owners.remove(&owner);
            }
        }
    }

    /// 清理过期频道，返回被清的 ID（调用方据此收尾陈旧 waiter）。
    pub fn purge_expired(&self) -> Vec<u16> {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let expired: Vec<u16> = inner
            .channels
            .iter()
            .filter(|(_, ch)| ch.expires.is_some_and(|t| Instant::now() > t))
            .map(|(id, _)| *id)
            .collect();
        for id in &expired {
            Self::delete_locked(&mut inner, *id);
        }
        expired
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

    fn addr(ip: &str) -> SocketAddr {
        format!("{ip}:1234").parse().unwrap()
    }

    #[test]
    fn allocate_join_and_expiry_flow() {
        let store = MemoryStore::new(10);
        store
            .allocate_channel(1, Duration::from_secs(600), addr("203.0.113.1"))
            .unwrap();
        assert!(store.channel_exists(1));
        assert!(!store.channel_exists(2));
        // 重复分配报错
        assert!(
            store
                .allocate_channel(1, Duration::from_secs(600), addr("203.0.113.1"))
                .is_err()
        );
        // blob 存取 + 过期语义
        store.store_blob(1, true, b"blob3").unwrap();
        assert_eq!(
            store.fetch_blob(1, true).unwrap().as_deref(),
            Some(b"blob3".as_slice())
        );
        assert_eq!(
            store.fetch_blob(1, false).unwrap().as_deref(),
            Some([].as_slice())
        );
        assert!(store.fetch_blob(2, true).unwrap().is_none());
    }

    #[test]
    fn per_ip_channel_cap_enforced_and_released() {
        let store = MemoryStore::new(2);
        store
            .allocate_channel(1, Duration::from_secs(600), addr("203.0.113.1"))
            .unwrap();
        store
            .allocate_channel(2, Duration::from_secs(600), addr("203.0.113.1"))
            .unwrap();
        assert!(
            store
                .allocate_channel(3, Duration::from_secs(600), addr("203.0.113.1"))
                .is_err(),
            "超过每 IP 上限必须拒绝"
        );
        store
            .allocate_channel(3, Duration::from_secs(600), addr("203.0.113.2"))
            .unwrap();
        // 删除一个后名额归还
        store.delete_channel(1);
        store
            .allocate_channel(4, Duration::from_secs(600), addr("203.0.113.1"))
            .unwrap();
    }

    #[test]
    fn failures_count_and_purge() {
        let store = MemoryStore::new(0);
        store
            .allocate_channel(9, Duration::from_secs(600), addr("203.0.113.1"))
            .unwrap();
        assert_eq!(store.record_failure(9).unwrap(), 1);
        assert_eq!(store.record_failure(9).unwrap(), 2);
        assert!(store.record_failure(10).is_err());
        // 过期清空
        store
            .allocate_channel(11, Duration::ZERO, addr("203.0.113.1"))
            .unwrap();
        assert_eq!(store.purge_expired(), vec![11]);
        assert!(!store.channel_exists(11));
    }
}
