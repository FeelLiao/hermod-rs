//! 会话编排：从**传输码**到「已建立、可 listen/dial 的传输连接」。
//!
//! 完整链路（DEVELOPMENT.md §3.2 步骤 1–7）：
//!
//! ```text
//! 传输码 → 连 server（SPKI pinning）→ allocate/join → wait_ready
//!   → CPace 得 K → derive_wg_psk(K, channel_id)
//!   → prepare 传输（并入 server 观测地址 hint）
//!   → AEAD 加密交换端点 bundle（blob3/blob4）
//!   → establish 得到 WgConnection
//! ```
//!
//! tx 与 rx 只在「谁先发 bundle」上不同：
//! - tx：发 blob3（自己的 bundle）→ 收 blob4（对端 bundle）；
//! - rx：收 blob3 → 发 blob4。
//!
//! 传输码里的 `channelID-` 前缀只用于信令路由；CPace 的 password 是去掉前缀后的
//! 单词部分（与 Go 版一致）。

use crate::config::{self, Config};
use crate::crypto::cpace::Role;
use crate::crypto::kdf::{derive_http_token, derive_wg_psk};
use sha2::{Digest, Sha256};
use crate::crypto::wordlist;
use crate::signal::client::{self, ObservedAddrs, WsStream};
use crate::transport::iface::Transport;
use crate::transport::wireguard::session::{self as bundle, EndpointBundle};
use crate::transport::wireguard::transport::{WgConnection, WgError, WireGuardTransport};

/// 会话建立错误。
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("config: {0}")]
    Config(#[from] config::ConfigError),
    #[error("transfer code: {0}")]
    Code(#[from] wordlist::WordlistError),
    #[error("signaling: {0}")]
    Signal(#[from] client::ClientError),
    #[error("bundle: {0}")]
    Bundle(#[from] bundle::BundleError),
    #[error("bundle json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("transport: {0}")]
    Transport(#[from] WgError),
    #[error("server {url} 未被信任；先运行 `hermod trust {url}`")]
    Untrusted { url: String },
    #[error("配置里的 SPKI 指纹非法: {0}")]
    BadFingerprint(String),
    #[error("对端 bundle 的凭证指纹与本会话 token 不一致")]
    FingerprintMismatch,
}

fn sha256_32(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

/// tx 会话（已建立，`listen` 后发文件）。
pub struct TxSession {
    /// 已建立的 WireGuard 连接。
    pub conn: WgConnection,
    /// 隧道内 HTTP 的一次性 token（tx 用它拼 `GET /f/<token>`）。
    pub token: [u8; 16],
}

/// rx 会话（已建立，`dial` 后收文件）。
pub struct RxSession {
    /// 已建立的 WireGuard 连接。
    pub conn: WgConnection,
    /// 隧道内 HTTP 的一次性 token（rx 用同一个 token 拨号）。
    pub token: [u8; 16],
}

/// 配置里当前 server 的 SPKI 指纹（用于 `connect_pinned`）。
///
/// trust 存储的键是**规范化后的 server URL**（与 Go 版一致）。
pub fn expected_spki(config: &Config) -> Result<(String, [u8; 32]), SessionError> {
    let url = config::normalize_server_url(&config.server_url)?;
    let hex = config
        .trusted_servers
        .get(&url)
        .ok_or_else(|| SessionError::Untrusted { url: url.clone() })?;
    let fp = parse_hex32(hex).ok_or_else(|| SessionError::BadFingerprint(hex.clone()))?;
    Ok((url, fp))
}

fn parse_hex32(s: &str) -> Option<[u8; 32]> {
    let s = s.trim();
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

async fn open(config: &Config) -> Result<WsStream, SessionError> {
    let (url, spki) = expected_spki(config)?;
    Ok(client::connect_pinned(&url, spki).await?)
}

/// tx：从传输码开始，建立到 rx 的 WireGuard 连接。
pub async fn sender_session(config: &Config, code: &str) -> Result<TxSession, SessionError> {
    let (channel_id, words) = wordlist::parse_transfer_code(code)?;
    let password = words.join("-");
    let mut ws = open(config).await?;
    let (k, observed) = client::sender_handshake(&mut ws, channel_id, &password).await?;
    let token = derive_http_token(&k, channel_id);
    let conn =
        exchange_and_establish(&mut ws, channel_id, &k, &token, observed, Role::Sender).await?;
    // ws 在此 drop → 关闭 WebSocket，释放 server 资源（对应 Go 的显式 Close）
    Ok(TxSession { conn, token })
}

/// rx：从传输码开始，建立到 tx 的 WireGuard 连接。
pub async fn receiver_session(config: &Config, code: &str) -> Result<RxSession, SessionError> {
    let (channel_id, words) = wordlist::parse_transfer_code(code)?;
    let password = words.join("-");
    let mut ws = open(config).await?;
    let (k, observed) = client::receiver_handshake(&mut ws, channel_id, &password).await?;
    let token = derive_http_token(&k, channel_id);
    let conn =
        exchange_and_establish(&mut ws, channel_id, &k, &token, observed, Role::Receiver).await?;
    Ok(RxSession { conn, token })
}

/// 交换加密端点 bundle，并 establish 传输。
async fn exchange_and_establish(
    ws: &mut WsStream,
    channel_id: u16,
    k: &[u8; 32],
    token: &[u8; 16],
    observed: ObservedAddrs,
    role: Role,
) -> Result<WgConnection, SessionError> {
    let psk = derive_wg_psk(k, channel_id);
    let mut transport = WireGuardTransport::new();
    // server 观测地址作为 server-reflexive 提示（仅 IPv6 生效）
    transport.set_server_hint(observed.public_ipv6.parse().ok());
    let my_bytes = transport.prepare().await?;
    let mut my_bundle = EndpointBundle::decode(&my_bytes)?;
    // 把凭证指纹填成 SHA-256(token)，对端可据此校验 token 一致
    my_bundle.credential_fingerprint = sha256_32(token);
    let my_blob = bundle::seal_bundle(k, channel_id, &my_bundle)?;

    // tx 先发（blob3）再收（blob4）；rx 先收再发。顺序与 Go 版一致。
    let peer_bundle = match role {
        Role::Sender => {
            client::send_blob(ws, channel_id, &my_blob).await?;
            let peer_blob = client::recv_blob(ws).await?;
            bundle::open_bundle(k, channel_id, &peer_blob)?
        }
        Role::Receiver => {
            let peer_blob = client::recv_blob(ws).await?;
            client::send_blob(ws, channel_id, &my_blob).await?;
            bundle::open_bundle(k, channel_id, &peer_blob)?
        }
    };

    if peer_bundle.credential_fingerprint != sha256_32(token) {
        return Err(SessionError::FingerprintMismatch);
    }
    let peer_bytes = peer_bundle.encode()?;
    Ok(transport.establish(role, &psk, &peer_bytes).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with(url: &str, fp_hex: Option<&str>) -> Config {
        let trusted = match fp_hex {
            Some(fp) => std::collections::BTreeMap::from([(url.to_string(), fp.to_string())]),
            None => std::collections::BTreeMap::new(),
        };
        Config {
            server_url: url.to_string(),
            trusted_servers: trusted,
            ..Config::default()
        }
    }

    /// 已信任的 server → 读出规范化 URL + 指纹。
    #[test]
    fn expected_spki_reads_trusted_server() {
        let cfg = cfg_with("wss://relay.example.com:4376", Some(&"ab".repeat(32)));
        let (url, fp) = expected_spki(&cfg).unwrap();
        assert_eq!(url, "wss://relay.example.com:4376");
        assert_eq!(fp, [0xab; 32]);
    }

    /// 未信任 → 明确报错（提示去 trust）。
    #[test]
    fn untrusted_server_errors() {
        let cfg = cfg_with("wss://relay.example.com:4376", None);
        assert!(matches!(
            expected_spki(&cfg),
            Err(SessionError::Untrusted { .. })
        ));
    }

    /// 指纹长度/内容非法 → BadFingerprint。
    #[test]
    fn bad_fingerprint_errors() {
        let cfg = cfg_with("wss://relay.example.com:4376", Some("zz"));
        assert!(matches!(
            expected_spki(&cfg),
            Err(SessionError::BadFingerprint(_))
        ));
    }

    /// URL 规范化后再查 trust：server_url 不带端口，trust 键是规范化形式，也能命中。
    #[test]
    fn normalizes_url_before_lookup() {
        let normalized = config::normalize_server_url("wss://relay.example.com").unwrap();
        let cfg = Config {
            server_url: "wss://relay.example.com".to_string(),
            trusted_servers: std::collections::BTreeMap::from([(normalized, "cd".repeat(32))]),
            ..Config::default()
        };
        let (url, _) = expected_spki(&cfg).unwrap();
        assert_eq!(url, "wss://relay.example.com:4376");
    }
}
