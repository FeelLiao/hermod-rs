//! QUIC 自己的端点清单（endpoint bundle）。
//!
//! 与 WireGuard 的 `EndpointBundle` 完全无关：只有 QUIC 需要的字段。
//! 明文字节由 session 用 CPace 密钥 AEAD 加密后交换；对端解析后：
//!
//! 1. 校验 `version`；
//! 2. 校验 `credential_fingerprint == SHA-256(token)`；
//! 3. 用 `cert_fingerprint` 钉扎 QUIC server 的自签名证书；
//! 4. 用 `udp_port` + `ipv6_candidates` 发起连接。

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::net::Ipv6Addr;

/// 当前 QUIC endpoint 格式版本。
pub const QUIC_ENDPOINT_VERSION: u8 = 1;

/// QUIC 端点清单。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuicEndpoint {
    /// 格式版本；不符时直接拒绝。
    pub version: u8,
    /// QUIC endpoint 绑定的 UDP 端口。
    pub udp_port: u16,
    /// 本机 IPv6 候选地址（serde 不支持 `std::net` 类型，用字符串数组）。
    #[serde(
        serialize_with = "serialize_ipv6_list",
        deserialize_with = "deserialize_ipv6_list"
    )]
    pub ipv6_candidates: Vec<Ipv6Addr>,
    /// 可选局域网 IPv4 候选；旧端忽略，新端读取旧包时为空。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ipv4_candidates: Vec<std::net::Ipv4Addr>,
    /// 临时自签名证书 DER 的 SHA-256。
    pub cert_fingerprint: [u8; 32],
    /// 一次性凭证指纹：SHA-256(HTTP token)。
    pub credential_fingerprint: [u8; 32],
}

impl QuicEndpoint {
    /// 序列化为 JSON 字节（随后会被 session AEAD 加密）。
    pub fn encode(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    /// 从 JSON 字节解析。
    pub fn decode(data: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(data)
    }
}

fn serialize_ipv6_list<S: Serializer>(addrs: &[Ipv6Addr], s: S) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    let mut seq = s.serialize_seq(Some(addrs.len()))?;
    for addr in addrs {
        seq.serialize_element(&addr.to_string())?;
    }
    seq.end()
}

fn deserialize_ipv6_list<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Ipv6Addr>, D::Error> {
    let strs = Vec::<String>::deserialize(d)?;
    strs.into_iter()
        .map(|s| s.parse().map_err(serde::de::Error::custom))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> QuicEndpoint {
        QuicEndpoint {
            version: QUIC_ENDPOINT_VERSION,
            udp_port: 4433,
            ipv4_candidates: Vec::new(),
            ipv6_candidates: vec!["2001:db8::1".parse().unwrap(), "fd00::1".parse().unwrap()],
            cert_fingerprint: [0x11; 32],
            credential_fingerprint: [0x22; 32],
        }
    }

    #[test]
    fn encode_decode_roundtrip() {
        let b = sample();
        let data = b.encode().unwrap();
        assert_eq!(QuicEndpoint::decode(&data).unwrap(), b);
    }

    #[test]
    fn json_contains_quic_fields_only() {
        let data = sample().encode().unwrap();
        let s = String::from_utf8(data).unwrap();
        assert!(s.contains("\"udp_port\""));
        assert!(s.contains("\"ipv6_candidates\""));
        assert!(s.contains("\"cert_fingerprint\""));
        assert!(s.contains("\"credential_fingerprint\""));
        assert!(!s.contains("wg_public_key"));
        assert!(!s.contains("http_port"));
    }
}
