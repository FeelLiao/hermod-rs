//! 持久设备身份、规范设备 ID 与带域分隔的 P-256 签名。

pub(crate) mod storage;

use p256::ecdsa::{
    Signature, SigningKey, VerifyingKey,
    signature::{Signer, Verifier},
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::{fmt, path::Path, str::FromStr};

pub use storage::{Error, Result};

/// SHA-256（规范压缩公钥）的前 16 字节；文本严格为 32 位小写十六进制。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeviceId([u8; 16]);

impl DeviceId {
    /// 验证规范压缩 P-256 公钥并计算设备 ID。
    pub fn from_public_key(public_key: &[u8]) -> Result<Self> {
        canonical_key(public_key)?;
        let digest = Sha256::digest(public_key);
        let mut id = [0; 16];
        id.copy_from_slice(&digest[..16]);
        Ok(Self(id))
    }
}
impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}
impl fmt::Debug for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl FromStr for DeviceId {
    type Err = Error;
    fn from_str(value: &str) -> Result<Self> {
        Ok(Self(storage::decode_hex::<16>(value)?))
    }
}
impl Serialize for DeviceId {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}
impl<'de> Deserialize<'de> for DeviceId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// 不公开私钥、不自动轮换的设备身份。
pub struct Identity {
    key: SigningKey,
    id: DeviceId,
}
impl fmt::Debug for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Identity")
            .field("device_id", &self.id)
            .finish_non_exhaustive()
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IdentityFile {
    version: u32,
    device_id: DeviceId,
    public_key: String,
    private_key: String,
}
impl Identity {
    /// 在跨进程锁内读取或首次生成；损坏、版本或权限错误不会生成替代身份。
    pub fn load_or_create(dir: &Path) -> Result<Self> {
        let dir = dir.join("device-state");
        storage::prepare_dir(&dir)?;
        let _lock = storage::lock(&dir, "identity.lock")?;
        let path = dir.join("device.json");
        if let Some(bytes) = storage::read(&path)? {
            let stored: IdentityFile = storage::parse_json(&bytes)?;
            if stored.version != 1 {
                return Err(Error::Invalid("unsupported identity version"));
            }
            let private = storage::decode_hex::<32>(&stored.private_key)?;
            let key = SigningKey::from_slice(&private)
                .map_err(|_| Error::Invalid("invalid private key"))?;
            let public = storage::decode_hex::<33>(&stored.public_key)?;
            let id = DeviceId::from_public_key(&public)?;
            if key.verifying_key().to_sec1_point(true).as_bytes() != public
                || id != stored.device_id
            {
                return Err(Error::Invalid("identity key/id mismatch"));
            }
            return Ok(Self { key, id });
        }
        let key = loop {
            let mut bytes = [0; 32];
            getrandom::fill(&mut bytes).map_err(|_| Error::Invalid("random source failed"))?;
            if let Ok(key) = SigningKey::from_slice(&bytes) {
                break key;
            }
        };
        let public = key.verifying_key().to_sec1_point(true);
        let id = DeviceId::from_public_key(public.as_bytes())?;
        let stored = IdentityFile {
            version: 1,
            device_id: id,
            public_key: storage::hex(public.as_bytes()),
            private_key: storage::hex(&key.to_bytes()),
        };
        storage::atomic_json(&path, &stored)?;
        Ok(Self { key, id })
    }
    pub fn device_id(&self) -> DeviceId {
        self.id
    }
    pub fn public_key(&self) -> [u8; 33] {
        let mut out = [0; 33];
        out.copy_from_slice(self.key.verifying_key().to_sec1_point(true).as_bytes());
        out
    }
    /// 签名消息：固定协议前缀、域长度（u64 BE）、域、消息长度（u64 BE）、消息。
    pub fn sign(&self, domain: &[u8], message: &[u8]) -> Result<[u8; 64]> {
        let signature: Signature = self.key.sign(&signing_message(domain, message)?);
        Ok(signature.to_bytes().into())
    }
}
fn signing_message(domain: &[u8], message: &[u8]) -> Result<Vec<u8>> {
    if domain.is_empty() {
        return Err(Error::Invalid("empty signature domain"));
    }
    let mut bytes = b"hermod/device-signature/v1\0".to_vec();
    bytes.extend_from_slice(
        &u64::try_from(domain.len())
            .map_err(|_| Error::Invalid("domain too long"))?
            .to_be_bytes(),
    );
    bytes.extend_from_slice(domain);
    bytes.extend_from_slice(
        &u64::try_from(message.len())
            .map_err(|_| Error::Invalid("message too long"))?
            .to_be_bytes(),
    );
    bytes.extend_from_slice(message);
    Ok(bytes)
}
fn canonical_key(public_key: &[u8]) -> Result<VerifyingKey> {
    if public_key.len() != 33 || !matches!(public_key[0], 2 | 3) {
        return Err(Error::Invalid("noncanonical public key"));
    }
    let key = VerifyingKey::from_sec1_bytes(public_key)
        .map_err(|_| Error::Invalid("invalid public key"))?;
    if key.to_sec1_point(true).as_bytes() != public_key {
        return Err(Error::Invalid("noncanonical public key"));
    }
    Ok(key)
}
/// 无需加载本机身份即可验证；只接受规范公钥和固定 64 字节签名。
pub fn verify(public_key: &[u8], domain: &[u8], message: &[u8], signature: &[u8]) -> Result<()> {
    let key = canonical_key(public_key)?;
    let signature = Signature::from_slice(signature)
        .map_err(|_| Error::Invalid("invalid signature encoding"))?;
    key.verify(&signing_message(domain, message)?, &signature)
        .map_err(|_| Error::Invalid("signature verification failed"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_framing_is_unambiguous_and_stable() {
        let bytes = signing_message(b"ab", b"c").unwrap();
        let mut expected = b"hermod/device-signature/v1\0".to_vec();
        expected.extend_from_slice(&2_u64.to_be_bytes());
        expected.extend_from_slice(b"ab");
        expected.extend_from_slice(&1_u64.to_be_bytes());
        expected.extend_from_slice(b"c");
        assert_eq!(bytes, expected);
        assert_ne!(bytes, signing_message(b"a", b"bc").unwrap());
    }
}
