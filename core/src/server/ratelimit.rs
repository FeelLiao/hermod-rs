//! 每 IP 前缀的令牌桶限流器（对应 Go `ratelimit.go`）。
//!
//! 桶键是 `HMAC-SHA256(日盐, IP 前缀)`：防止把真实 IP 留在内存里被人看见
//! （按 UTC 日轮换盐，轮换时清空桶；对应 Go 的防追踪设计）。

use hmac::{Hmac, KeyInit, Mac};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

type HmacSha256 = Hmac<sha2::Sha256>;

/// 令牌桶：`rate` 个/秒，容量 `burst`。
pub struct RateLimiter {
    inner: Mutex<Inner>,
    rate: f64,
    burst: f64,
}

struct Inner {
    buckets: HashMap<String, Bucket>,
    /// 当前 HMAC 盐。
    salt: [u8; 32],
    /// 盐生成时的 UTC 日序（UNIX_EPOCH 起的天数）。
    salt_day: u64,
}

#[derive(Clone, Copy)]
struct Bucket {
    tokens: f64,
    last_seen: Instant,
}

fn utc_day() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        / 86_400
}

impl RateLimiter {
    /// 新建限流器；立即生成一把密码学盐，按 UTC 日轮换。
    pub fn new(rate: f64, burst: f64) -> Self {
        let mut salt = [0u8; 32];
        rand_core::Rng::fill_bytes(&mut rand_core::UnwrapErr(getrandom::SysRng), &mut salt);
        Self {
            inner: Mutex::new(Inner {
                buckets: HashMap::new(),
                salt,
                salt_day: utc_day(),
            }),
            rate,
            burst,
        }
    }

    /// 该请求是否放行（addr 为远端地址）。
    pub fn allow(&self, addr: SocketAddr) -> bool {
        let prefix = ip_prefix(addr);
        let now = Instant::now();
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // 跨 UTC 日：换盐并清空所有桶（轮换失败不可能——盐是本地生成）。
        let today = utc_day();
        if today > inner.salt_day {
            rand_core::Rng::fill_bytes(
                &mut rand_core::UnwrapErr(getrandom::SysRng),
                &mut inner.salt,
            );
            inner.salt_day = today;
            inner.buckets.clear();
        }
        let key = hash_prefix(&inner.salt, &prefix);
        let bucket = inner.buckets.entry(key).or_insert(Bucket {
            tokens: self.burst,
            last_seen: now,
        });
        let elapsed = now.duration_since(bucket.last_seen).as_secs_f64();
        bucket.last_seen = now;
        bucket.tokens = (bucket.tokens + elapsed * self.rate).min(self.burst);
        if bucket.tokens < 1.0 {
            return false;
        }
        bucket.tokens -= 1.0;
        true
    }

    /// 清理 `max_age` 未活跃的桶（防每个源 IP 撑大内存）。
    pub fn cleanup(&self, max_age: Duration) {
        let cutoff = Instant::now() - max_age;
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.buckets.retain(|_, b| b.last_seen > cutoff);
    }
}

/// 桶键：hex(HMAC-SHA256(salt, prefix))。
fn hash_prefix(salt: &[u8; 32], prefix: &str) -> String {
    let mut mac =
        <HmacSha256 as KeyInit>::new_from_slice(salt).expect("HMAC accepts any key length");
    mac.update(prefix.as_bytes());
    let tag = mac.finalize().into_bytes();
    tag.iter().map(|b| format!("{b:02x}")).collect()
}

/// 地址的网络前缀：IPv4 用完整 /32，IPv6 用 /64（对应 Go `ipPrefix`）。
pub fn ip_prefix(addr: SocketAddr) -> String {
    match addr.ip() {
        std::net::IpAddr::V4(ip) => ip.to_string(),
        std::net::IpAddr::V6(ip) => {
            let octets = ip.octets();
            let mut masked = [0u8; 16];
            masked[..8].copy_from_slice(&octets[..8]);
            std::net::Ipv6Addr::from(masked).to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv6Addr;

    #[test]
    fn burst_is_enforced_then_recovers() {
        let limiter = RateLimiter::new(1000.0, 3.0);
        let addr: SocketAddr = "203.0.113.7:4444".parse().unwrap();
        assert!(limiter.allow(addr));
        assert!(limiter.allow(addr));
        assert!(limiter.allow(addr));
        // 突发用尽；速率 1000/s 下等一小段就恢复
        assert!(!limiter.allow(addr));
        std::thread::sleep(Duration::from_millis(5));
        assert!(limiter.allow(addr));
    }

    #[test]
    fn ipv6_prefix_masks_to_slash64() {
        let v6: SocketAddr = "[2001:db8:1:2:3:4:5:6]:443".parse().unwrap();
        let v6_same_prefix: SocketAddr = "[2001:db8:1:2:ffff::1]:1".parse().unwrap();
        let v6_other_prefix: SocketAddr = "[2001:db8:1:3::1]:1".parse().unwrap();
        assert_eq!(ip_prefix(v6), ip_prefix(v6_same_prefix));
        assert_ne!(ip_prefix(v6), ip_prefix(v6_other_prefix));
        // IPv4 是完整 /32
        let v4a: SocketAddr = "203.0.113.1:1".parse().unwrap();
        let v4b: SocketAddr = "203.0.113.2:1".parse().unwrap();
        assert_ne!(ip_prefix(v4a), ip_prefix(v4b));
    }

    #[test]
    fn cleanup_removes_stale_buckets() {
        let limiter = RateLimiter::new(1.0, 1.0);
        let addr: SocketAddr = "198.51.100.9:8".parse().unwrap();
        assert!(limiter.allow(addr));
        limiter.cleanup(Duration::ZERO);
        let inner = limiter.inner.lock().unwrap();
        assert!(inner.buckets.is_empty());
    }

    #[test]
    fn loopback_v6_prefix_is_stable() {
        let a: SocketAddr = "[::1]:1".parse().unwrap();
        let b: SocketAddr = SocketAddr::new(Ipv6Addr::LOCALHOST.into(), 2);
        assert_eq!(ip_prefix(a), ip_prefix(b));
    }
}
