//! 联系人摘要、秘密配对状态和跨进程短事务。
use crate::identity::{
    DeviceId,
    storage::{self, Error, Result},
};
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// 配对候选只有确认后才能用于认证。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairingState {
    Pending,
    Active,
}

/// 无秘密、可安全显示的联系人快照；时间戳为 Unix 秒。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contact {
    pub device_id: DeviceId,
    pub public_key: [u8; 33],
    pub name: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub pairing_id: String,
    pub state: PairingState,
}
/// 仅通过显式访问器向会话层提供秘密，不提供序列化或公开字段。
#[derive(Clone)]
pub struct Pairing {
    contact: Contact,
    secret: [u8; 32],
}
impl fmt::Debug for Pairing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pairing")
            .field("contact", &self.contact)
            .field("secret", &"[REDACTED]")
            .finish()
    }
}
impl Pairing {
    pub fn contact(&self) -> &Contact {
        &self.contact
    }
    /// 长期配对秘密；调用方不可打印或向日志格式化此字节数组。
    pub fn secret(&self) -> &[u8; 32] {
        &self.secret
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    device_id: DeviceId,
    public_key: String,
    name: String,
    created_at: u64,
    updated_at: u64,
    pairing_id: String,
    state: PairingState,
    secret: String,
}
impl Record {
    fn pairing(&self) -> Result<Pairing> {
        let public_key = storage::decode_hex::<33>(&self.public_key)?;
        if DeviceId::from_public_key(&public_key)? != self.device_id {
            return Err(Error::Invalid("contact key/id mismatch"));
        }
        validate_name(&self.name)?;
        validate_pairing_id(&self.pairing_id)?;
        if self.updated_at < self.created_at {
            return Err(Error::Invalid("invalid contact timestamps"));
        }
        let secret = storage::decode_hex::<32>(&self.secret)?;
        Ok(Pairing {
            contact: Contact {
                device_id: self.device_id,
                public_key,
                name: self.name.clone(),
                created_at: self.created_at,
                updated_at: self.updated_at,
                pairing_id: self.pairing_id.clone(),
                state: self.state,
            },
            secret,
        })
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContactsFile {
    version: u32,
    entries: Vec<Record>,
}
impl ContactsFile {
    fn validate(&self) -> Result<()> {
        if self.version != 1 {
            return Err(Error::Invalid("unsupported contacts version"));
        }
        let mut ids = std::collections::HashSet::new();
        let mut pairing_ids = std::collections::HashSet::new();
        for entry in &self.entries {
            entry.pairing()?;
            if !ids.insert(entry.device_id) || !pairing_ids.insert(&entry.pairing_id) {
                return Err(Error::Invalid("duplicate contact or pairing id"));
            }
        }
        Ok(())
    }
}
fn now() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::Invalid("system clock before epoch"))?
        .as_secs())
}
fn validate_name(name: &str) -> Result<()> {
    if name.len() > 256 || name.chars().any(char::is_control) {
        return Err(Error::Invalid("invalid contact name"));
    }
    Ok(())
}
fn validate_pairing_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err(Error::Invalid("invalid pairing id"));
    }
    Ok(())
}
/// 可克隆的存储句柄，不缓存记录，避免并发进程丢失更新。
#[derive(Debug, Clone)]
pub struct Contacts {
    dir: PathBuf,
}
/// 同目录独立发送租约；丢弃时由文件句柄释放 OS 锁。
#[derive(Debug)]
pub struct SendLease {
    _lock: storage::Lock,
}
impl Contacts {
    pub fn open(dir: &Path) -> Result<Self> {
        let dir = dir.join("device-state");
        storage::prepare_dir(&dir)?;
        let contacts = Self {
            dir: std::path::absolute(dir)?,
        };
        contacts.transaction(|_| Ok(()))?;
        Ok(contacts)
    }
    fn transaction<T>(&self, operation: impl FnOnce(&mut ContactsFile) -> Result<T>) -> Result<T> {
        let _lock = storage::lock(&self.dir, "contacts.lock")?;
        let path = self.dir.join("contacts.json");
        let bytes = storage::read(&path)?;
        let mut stored = match &bytes {
            Some(bytes) => storage::parse_json(bytes)?,
            None => ContactsFile {
                version: 1,
                entries: Vec::new(),
            },
        };
        stored.validate()?;
        let previous =
            serde_json::to_vec(&stored).map_err(|_| Error::Invalid("serialization failed"))?;
        let result = operation(&mut stored)?;
        stored.validate()?;
        let changed = previous
            != serde_json::to_vec(&stored).map_err(|_| Error::Invalid("serialization failed"))?;
        if bytes.is_none() || changed {
            storage::atomic_json(&path, &stored)?;
        }
        Ok(result)
    }
    /// 包含 pending 摘要，但不会暴露秘密。
    pub fn list(&self) -> Result<Vec<Contact>> {
        self.transaction(|stored| {
            stored
                .entries
                .iter()
                .map(|entry| Ok(entry.pairing()?.contact))
                .collect()
        })
    }
    /// 仅查找 active 配对；缺失与损坏严格区分。
    pub fn find(&self, id: DeviceId) -> Result<Option<Pairing>> {
        self.transaction(|stored| {
            stored
                .entries
                .iter()
                .find(|entry| entry.device_id == id && entry.state == PairingState::Active)
                .map(Record::pairing)
                .transpose()
        })
    }
    /// 恢复确认事务时读取 pending 或 active，缺失不吞掉存储错误。
    pub fn find_any(&self, id: DeviceId) -> Result<Option<Pairing>> {
        self.transaction(|stored| {
            stored
                .entries
                .iter()
                .find(|entry| entry.device_id == id)
                .map(Record::pairing)
                .transpose()
        })
    }
    pub fn lookup(&self, id: DeviceId) -> Result<Pairing> {
        self.find(id)?.ok_or(Error::NotFound)
    }
    /// 规范 ID 优先（即使不存在也不回退备注）；否则精确匹配 active 备注。
    pub fn resolve(&self, value: &str) -> Result<Pairing> {
        if let Ok(id) = value.parse() {
            return self.lookup(id);
        }
        self.transaction(|stored| {
            let mut entries = stored
                .entries
                .iter()
                .filter(|entry| entry.state == PairingState::Active && entry.name == value);
            let first = entries.next().ok_or(Error::NotFound)?;
            if entries.next().is_some() {
                return Err(Error::Ambiguous);
            }
            first.pairing()
        })
    }
    pub fn rename(&self, id: DeviceId, name: &str) -> Result<()> {
        validate_name(name)?;
        self.transaction(|stored| {
            let entry = stored
                .entries
                .iter_mut()
                .find(|entry| entry.device_id == id)
                .ok_or(Error::NotFound)?;
            if entry.name != name {
                entry.name = name.into();
                entry.updated_at = now()?.max(entry.updated_at);
            }
            Ok(())
        })
    }
    pub fn remove(&self, id: DeviceId) -> Result<()> {
        self.transaction(|stored| {
            let index = stored
                .entries
                .iter()
                .position(|entry| entry.device_id == id)
                .ok_or(Error::NotFound)?;
            stored.entries.remove(index);
            Ok(())
        })
    }
    /// 保存候选但不激活；只允许完全相同的 pairing_id/公钥/秘密幂等重试。
    pub fn stage_pairing(
        &self,
        id: DeviceId,
        public_key: [u8; 33],
        name: &str,
        pairing_id: &str,
        secret: [u8; 32],
    ) -> Result<()> {
        if DeviceId::from_public_key(&public_key)? != id {
            return Err(Error::Invalid("contact key/id mismatch"));
        }
        validate_name(name)?;
        validate_pairing_id(pairing_id)?;
        self.transaction(|stored| {
            if let Some(entry) = stored
                .entries
                .iter()
                .find(|entry| entry.device_id == id || entry.pairing_id == pairing_id)
            {
                let pairing = entry.pairing()?;
                if entry.device_id == id
                    && entry.pairing_id == pairing_id
                    && pairing.contact.public_key == public_key
                    && pairing.secret == secret
                {
                    return Ok(());
                }
                return Err(Error::Conflict);
            }
            let timestamp = now()?;
            stored.entries.push(Record {
                device_id: id,
                public_key: storage::hex(&public_key),
                name: name.into(),
                created_at: timestamp,
                updated_at: timestamp,
                pairing_id: pairing_id.into(),
                state: PairingState::Pending,
                secret: storage::hex(&secret),
            });
            Ok(())
        })
    }
    /// 只激活精确匹配的候选；同 pairing_id 已激活时为幂等成功。
    pub fn confirm_pairing(&self, id: DeviceId, pairing_id: &str) -> Result<()> {
        self.transaction(|stored| {
            let entry = stored
                .entries
                .iter_mut()
                .find(|entry| entry.device_id == id)
                .ok_or(Error::NotFound)?;
            if entry.pairing_id != pairing_id {
                return Err(Error::Conflict);
            }
            if entry.state == PairingState::Pending {
                entry.state = PairingState::Active;
                entry.updated_at = now()?.max(entry.updated_at);
            }
            Ok(())
        })
    }
    /// 尝试获取发送租约；已占用时立即返回 Busy，不等待另一发送完成。
    /// 不持有联系人事务锁，网络调用期间只保留此句柄。
    pub fn acquire_send_lease(&self) -> Result<SendLease> {
        Ok(SendLease {
            _lock: storage::try_lock(&self.dir, "send.lock")?,
        })
    }
}
