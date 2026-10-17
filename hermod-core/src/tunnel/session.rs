//! 会话编排的 bundle 层（M2.4）：加密的端点清单交换。
//!
//! 对应 DEVELOPMENT.md §3.2 步骤 5。**注意**：Rust 版 bundle 内容与 Go 不同
//! （Go 公布临时 TLS 证书指纹，Rust 公布 WireGuard 公钥，因为数据面不同），
//! 所以本模块**不照抄 Go 的 handshake.go**，按 DESIGN 定义自己的格式（M2.4 提示）。
//!
//! 流程：双方各生成 [`EndpointBundle`] → `encode` 成 JSON → [`seal_bundle`]
//! 用 M1.3 的 AEAD 加密（AAD = 频道号 2 字节大端）→ 作为 blob 经 server 中继。

use crate::crypto::aead::{self, AeadError};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::net::Ipv6Addr;

/// 当前 bundle 协议版本。
pub const BUNDLE_VERSION: u8 = 1;

/// 端点清单：双方经加密 blob 交换的会合信息（DEVELOPMENT.md §3.2 步骤 5）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointBundle {
    /// 协议版本。不符时 [`open_bundle`] 报明确错误，避免新旧协议错配。
    pub version: u8,
    /// 本端 WireGuard 公钥（X25519，32 字节）。
    pub wg_public_key: [u8; 32],
    /// 绑定的 UDP 端口（WireGuard 监听）。
    pub udp_port: u16,
    /// HTTP 端口（tx 在这里起 HTTP server；双方都在 bundle 里公布，角色决定谁用）。
    pub http_port: u16,
    /// 本地 IPv6 候选地址。serde 不支持 `std::net` 类型，用自定义字符串序列化。
    #[serde(
        serialize_with = "serialize_ipv6_list",
        deserialize_with = "deserialize_ipv6_list"
    )]
    pub ipv6_candidates: Vec<Ipv6Addr>,
    /// 一次性凭证指纹：SHA-256(一次性 URL token)，防误连/串流（§3.4）。
    pub credential_fingerprint: [u8; 32],
}

impl EndpointBundle {
    /// 序列化为 JSON 字节（bundle 的明文格式；随后会被 AEAD 加密）。
    pub fn encode(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    /// 从 JSON 字节解析。
    pub fn decode(data: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(data)
    }
}

/// 用 CPace 共享密钥加密并序列化 bundle：AAD = 频道号（2 字节大端，与
/// cpace 的 DST、kdf 的 salt 用同一编码）。
pub fn seal_bundle(
    key: &[u8; 32],
    channel_id: u16,
    bundle: &EndpointBundle,
) -> Result<Vec<u8>, BundleError> {
    let plaintext = bundle.encode().map_err(BundleError::Encode)?;
    Ok(aead::seal(key, &channel_id.to_be_bytes(), &plaintext)?)
}

/// 解密并解析 bundle；版本不符时给出明确错误（M2.4 验收项）。
pub fn open_bundle(
    key: &[u8; 32],
    channel_id: u16,
    blob: &[u8],
) -> Result<EndpointBundle, BundleError> {
    let plaintext = aead::open(key, &channel_id.to_be_bytes(), blob)?;
    let bundle = EndpointBundle::decode(&plaintext).map_err(BundleError::Decode)?;
    if bundle.version != BUNDLE_VERSION {
        return Err(BundleError::VersionMismatch {
            got: bundle.version,
            expected: BUNDLE_VERSION,
        });
    }
    Ok(bundle)
}

/// 把 IPv6 地址列表序列化成字符串数组（serde 不实现 `std::net` 类型）。
fn serialize_ipv6_list<S: Serializer>(addrs: &[Ipv6Addr], s: S) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    let mut seq = s.serialize_seq(Some(addrs.len()))?;
    for addr in addrs {
        seq.serialize_element(&addr.to_string())?;
    }
    seq.end()
}

/// 从字符串数组反序列化 IPv6 地址列表。
fn deserialize_ipv6_list<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Ipv6Addr>, D::Error> {
    let strs = Vec::<String>::deserialize(d)?;
    strs.into_iter()
        .map(|s| s.parse().map_err(serde::de::Error::custom))
        .collect()
}

/// Bundle 层错误。
#[derive(Debug, thiserror::Error)]
pub enum BundleError {
    #[error("serialize bundle: {0}")]
    Encode(#[source] serde_json::Error),
    #[error("deserialize bundle: {0}")]
    Decode(#[source] serde_json::Error),
    #[error("bundle AEAD: {0}")]
    Aead(#[from] AeadError),
    #[error("bundle version mismatch: got {got}, expected {expected}")]
    VersionMismatch { got: u8, expected: u8 },
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [0x11; 32];

    fn sample_bundle() -> EndpointBundle {
        EndpointBundle {
            version: BUNDLE_VERSION,
            wg_public_key: [0x22; 32],
            udp_port: 51820,
            http_port: 48231,
            ipv6_candidates: vec![
                "2001:db8::1".parse().unwrap(),
                "fd00::1".parse().unwrap(),
                "fe80::1".parse().unwrap(),
            ],
            credential_fingerprint: [0x33; 32],
        }
    }

    /// JSON 编解码往返（含 IPv6 字符串化）。
    #[test]
    fn encode_decode_roundtrip() {
        let b = sample_bundle();
        let data = b.encode().unwrap();
        assert_eq!(EndpointBundle::decode(&data).unwrap(), b);
    }

    /// seal → open 往返，bundle 原样还原。
    #[test]
    fn seal_open_roundtrip() {
        let b = sample_bundle();
        let blob = seal_bundle(&KEY, 42, &b).unwrap();
        assert_eq!(open_bundle(&KEY, 42, &blob).unwrap(), b);
    }

    /// AAD（频道号）不对 / 密钥不对 → 必须拒绝。
    #[test]
    fn wrong_channel_or_key_fails() {
        let b = sample_bundle();
        let blob = seal_bundle(&KEY, 42, &b).unwrap();
        assert!(open_bundle(&KEY, 7, &blob).is_err()); // 频道号不同 → AAD 不匹配
        let other_key = [0x99; 32];
        assert!(open_bundle(&other_key, 42, &blob).is_err()); // 密钥不同
    }

    /// 密文被篡改 → 必须拒绝。
    #[test]
    fn tampered_fails() {
        let b = sample_bundle();
        let mut blob = seal_bundle(&KEY, 42, &b).unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 0x01;
        assert!(open_bundle(&KEY, 42, &blob).is_err());
    }

    /// 版本不符 → 明确错误（M2.4 验收项）。
    #[test]
    fn version_mismatch_is_explicit() {
        let mut b = sample_bundle();
        b.version = 99;
        let blob = seal_bundle(&KEY, 42, &b).unwrap();
        match open_bundle(&KEY, 42, &blob) {
            Err(BundleError::VersionMismatch { got, expected }) => {
                assert_eq!(got, 99);
                assert_eq!(expected, BUNDLE_VERSION);
            }
            other => panic!("expected VersionMismatch, got {other:?}"),
        }
    }
}
