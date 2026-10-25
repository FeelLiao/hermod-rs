//! 会话编排：从**传输码**到「已建立、可 listen/dial 的传输连接」。
//!
//! 完整链路（DEVELOPMENT.md §3.2 步骤 1–7）：
//!
//! ```text
//! 传输码 → 连 server（SPKI pinning）→ allocate/join → wait_ready
//!   → CPace 得 K → derive_wg_psk(K, channel_id)
//!   → tx 选传输（默认 WireGuard）→ prepare → 加端点头 → AEAD 交换 blob3
//!   → rx 解密 blob3 → 从端点头识别传输 → prepare → AEAD 回 blob4
//!   → 双方 establish 得到 AnyConnection
//! ```
//!
//! tx 与 rx 只在「谁先发端点」上不同：
//! - tx：发 blob3（自己的端点）→ 收 blob4（对端端点）；
//! - rx：收 blob3 → 发 blob4。
//!
//! 端点头格式（在 AEAD 明文里，不含敏感信息）：
//!
//! ```text
//! [0..2)  "HE"
//! [2]     header version (1)
//! [3]     transport wire code (1=wireguard, 2=quic)
//! [4..]   transport-specific opaque endpoint bytes
//! ```
//!
//! 传输码里的 `channelID-` 前缀只用于信令路由；CPace 的 password 是去掉前缀后的
//! 单词部分（与 Go 版一致）。

use crate::config::{self, Config};
use crate::crypto::aead::{self, AeadError};
use crate::crypto::cpace::Role;
use crate::crypto::kdf::{derive_http_token, derive_wg_psk};
use crate::crypto::wordlist;
use crate::signal::client::{self, ObservedAddrs, WsStream};
use crate::transport::iface::{Transport, TransportKind};
use crate::transport::{AnyConnection, AnyTransport, TransportError};
use sha2::{Digest, Sha256};

/// 端点头 magic。
const ENDPOINT_MAGIC: [u8; 2] = *b"HE";
/// 端点头版本。
const ENDPOINT_VERSION: u8 = 1;
/// 端点头长度（magic + version + transport）。
const ENDPOINT_HEADER_LEN: usize = 4;

/// 会话建立错误。
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("options: {0}")]
    Options(String),
    #[error("config: {0}")]
    Config(#[from] config::ConfigError),
    #[error("transfer code: {0}")]
    Code(#[from] wordlist::WordlistError),
    #[error("signaling: {0}")]
    Signal(#[from] client::ClientError),
    #[error("endpoint AEAD: {0}")]
    Aead(#[from] AeadError),
    #[error("transport: {0}")]
    Transport(#[from] TransportError),
    #[error("server {url} 未被信任；先运行 `hermod trust {url}`")]
    Untrusted { url: String },
    #[error("配置里的 SPKI 指纹非法: {0}")]
    BadFingerprint(String),
    #[error("bad endpoint header: {0}")]
    BadEndpointHeader(String),
    #[error("transport mismatch: local={local}, peer={peer}")]
    TransportMismatch {
        local: TransportKind,
        peer: TransportKind,
    },
}

fn sha256_32(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

/// 会话建立的可选参数（对应 CLI 的 `--transport` / `--mtu` / `--congestion` 等）。
#[derive(Debug, Clone)]
pub struct SessionOptions {
    /// 载荷限速，同时用于控制 QUIC 排队窗口。
    pub bandwidth_limit: crate::bandwidth::BandwidthLimit,
    /// 当前协议的 MTU：WireGuard 内层 IP（自动 1400），QUIC UDP 载荷上限（自动 1452）。
    pub mtu: Option<usize>,
    /// UDP 绑定地址（默认 `[::]:0`）。
    pub udp_bind: Option<std::net::SocketAddrV6>,
    /// 传输协议：
    /// - tx：`Some(kind)` 指定；`None` 等同于 QUIC。
    /// - rx：`None` = 自动跟随对端；`Some(kind)` = 强制并校验。
    pub transport: Option<TransportKind>,
    /// 统一拥塞控制；auto 在 WireGuard 下为 CUBIC，在 QUIC 下为 BBR。
    pub congestion: crate::options::Congestion,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            bandwidth_limit: crate::bandwidth::BandwidthLimit::UNLIMITED,
            mtu: None,
            udp_bind: None,
            // 默认 auto：接收端跟随发送端；发送端用 sender_default()。
            transport: None,
            congestion: crate::options::Congestion::Auto,
        }
    }
}

impl SessionOptions {
    /// 发送端默认：WireGuard。
    pub fn sender_default() -> Self {
        Self {
            transport: Some(TransportKind::WireGuard),
            ..Self::default()
        }
    }

    /// 接收端默认：不指定传输，自动跟随发送端。
    pub fn receiver_default() -> Self {
        Self::default()
    }
}

/// tx 会话（已建立，`listen` 后发文件）。
pub struct TxSession {
    /// 已建立的传输连接（WireGuard 或 QUIC）。
    pub conn: AnyConnection,
    /// 传输内 HTTP 的一次性 token（tx 用它拼 `GET /f/<token>`）。
    pub token: [u8; 16],
}

/// rx 会话（已建立，`dial` 后收文件）。
pub struct RxSession {
    /// 已建立的传输连接（WireGuard 或 QUIC）。
    pub conn: AnyConnection,
    /// 传输内 HTTP 的一次性 token（rx 用同一个 token 拨号）。
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
    if s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
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
    crate::log_info!(server = %url, "connecting to signaling server");
    let ws = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        client::connect_pinned(&url, spki),
    )
    .await
    .map_err(|_| {
        client::ClientError::Ws(tokio_tungstenite::tungstenite::Error::Io(
            std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "signaling connection timed out after 15s; check IPv6 connectivity and the server",
            ),
        ))
    })??;
    crate::log_info!("websocket connection established");
    Ok(ws)
}

/// tx：从传输码开始，建立到 rx 的传输连接（默认 WireGuard）。
pub async fn sender_session(config: &Config, code: &str) -> Result<TxSession, SessionError> {
    sender_session_with(config, code, &SessionOptions::sender_default()).await
}

/// tx：同 [`sender_session`]，可指定传输协议 / MTU / UDP 绑定地址。
pub async fn sender_session_with(
    config: &Config,
    code: &str,
    opts: &SessionOptions,
) -> Result<TxSession, SessionError> {
    sender_session_with_ready(config, code, opts, || {}).await
}

/// 信令频道创建成功后才公布传输码，GUI 可显示准确的等待状态。
pub async fn sender_session_with_ready(
    config: &Config,
    code: &str,
    opts: &SessionOptions,
    on_ready: impl FnOnce() + Send,
) -> Result<TxSession, SessionError> {
    opts.validate_for(opts.transport.unwrap_or(TransportKind::WireGuard))
        .map_err(SessionError::Options)?;
    let (channel_id, words) = wordlist::parse_transfer_code(code)?;
    let password = words.join("-");
    let mut ws = open(config).await?;
    crate::log_info!(channel_id, "waiting for receiver to join channel");
    let (k, observed) =
        client::sender_handshake_with_ready(&mut ws, channel_id, &password, on_ready).await?;
    crate::log_info!(
        public_v6 = %observed.public_ipv6,
        public_v4 = %observed.public_ipv4,
        "PAKE handshake complete"
    );
    let token = derive_http_token(&k, channel_id);
    let conn = exchange_and_establish(
        &mut ws,
        channel_id,
        &k,
        &token,
        observed,
        Role::Sender,
        opts,
    )
    .await?;
    // ws 在此 drop → 关闭 WebSocket，释放 server 资源（对应 Go 的显式 Close）
    Ok(TxSession { conn, token })
}

/// rx：从传输码开始，建立到 tx 的传输连接（默认自动跟随发送端）。
pub async fn receiver_session(config: &Config, code: &str) -> Result<RxSession, SessionError> {
    receiver_session_with(config, code, &SessionOptions::receiver_default()).await
}

/// rx：同 [`receiver_session`]，可强制传输协议 / 指定 MTU / UDP 绑定地址。
pub async fn receiver_session_with(
    config: &Config,
    code: &str,
    opts: &SessionOptions,
) -> Result<RxSession, SessionError> {
    if let Some(kind) = opts.transport {
        opts.validate_for(kind).map_err(SessionError::Options)?;
    }
    let (channel_id, words) = wordlist::parse_transfer_code(code)?;
    let password = words.join("-");
    let mut ws = open(config).await?;
    let (k, observed) = client::receiver_handshake(&mut ws, channel_id, &password).await?;
    crate::log_info!(
        public_v6 = %observed.public_ipv6,
        public_v4 = %observed.public_ipv4,
        "PAKE handshake complete"
    );
    let token = derive_http_token(&k, channel_id);
    let conn = exchange_and_establish(
        &mut ws,
        channel_id,
        &k,
        &token,
        observed,
        Role::Receiver,
        opts,
    )
    .await?;
    Ok(RxSession { conn, token })
}

/// 构造指定传输实现，并应用所有通用/协议特有选项。
fn build_transport(
    opts: &SessionOptions,
    kind: TransportKind,
    observed: &ObservedAddrs,
    fingerprint: [u8; 32],
) -> Result<AnyTransport, SessionError> {
    opts.validate_for(kind).map_err(SessionError::Options)?;
    let mut transport = AnyTransport::new(kind);
    transport.set_udp_bind(opts.udp_bind);
    transport.configure(opts.mtu, opts.congestion);
    transport.set_bandwidth_limit(opts.bandwidth_limit);
    // server 观测地址作为 server-reflexive 提示（仅 IPv6 生效）
    transport.set_server_hint(observed.public_ipv6.parse().ok());
    transport.set_credential_fingerprint(fingerprint);
    Ok(transport)
}

/// 在 opaque endpoint 前加一个小头，让接收端能识别发送端使用的传输。
fn wrap_endpoint(kind: TransportKind, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(ENDPOINT_HEADER_LEN + payload.len());
    out.extend_from_slice(&ENDPOINT_MAGIC);
    out.push(ENDPOINT_VERSION);
    out.push(kind.to_wire());
    out.extend_from_slice(payload);
    out
}

/// 解析端点头，返回传输类型与 opaque payload。
fn parse_endpoint(bytes: &[u8]) -> Result<(TransportKind, &[u8]), SessionError> {
    if bytes.len() < ENDPOINT_HEADER_LEN {
        return Err(SessionError::BadEndpointHeader(format!(
            "too short: {} bytes",
            bytes.len()
        )));
    }
    if bytes[..2] != ENDPOINT_MAGIC {
        return Err(SessionError::BadEndpointHeader("bad magic".into()));
    }
    if bytes[2] != ENDPOINT_VERSION {
        return Err(SessionError::BadEndpointHeader(format!(
            "unsupported version {}",
            bytes[2]
        )));
    }
    let kind = TransportKind::from_wire(bytes[3]).ok_or_else(|| {
        SessionError::BadEndpointHeader(format!("unknown transport {}", bytes[3]))
    })?;
    Ok((kind, &bytes[ENDPOINT_HEADER_LEN..]))
}

/// 交换加密端点字节，并 establish 传输。
///
/// tx 先 prepare+send，因为接收端需要先看到 tx 的端点头才能选传输实现。
async fn exchange_and_establish(
    ws: &mut WsStream,
    channel_id: u16,
    k: &[u8; 32],
    token: &[u8; 16],
    observed: ObservedAddrs,
    role: Role,
    opts: &SessionOptions,
) -> Result<AnyConnection, SessionError> {
    let fingerprint = sha256_32(token);
    let psk = derive_wg_psk(k, channel_id);
    let aad = channel_id.to_be_bytes();

    match role {
        Role::Sender => {
            let kind = opts.transport.unwrap_or(TransportKind::WireGuard);
            let mut transport = build_transport(opts, kind, &observed, fingerprint)?;
            let my_endpoint = transport.prepare().await?;
            let my_blob = aead::seal(k, &aad, &wrap_endpoint(kind, &my_endpoint))?;
            client::send_blob(ws, channel_id, &my_blob).await?;

            let peer_blob = client::recv_blob(ws).await?;
            let peer_plain = aead::open(k, &aad, &peer_blob)?;
            let (peer_kind, peer_endpoint) = parse_endpoint(&peer_plain)?;
            if peer_kind != kind {
                return Err(SessionError::TransportMismatch {
                    local: kind,
                    peer: peer_kind,
                });
            }

            crate::log_info!(transport = %kind, "endpoint exchanged, establishing transport");
            Ok(transport.establish(role, &psk, peer_endpoint).await?)
        }
        Role::Receiver => {
            let peer_blob = client::recv_blob(ws).await?;
            let peer_plain = aead::open(k, &aad, &peer_blob)?;
            let (peer_kind, peer_endpoint) = parse_endpoint(&peer_plain)?;
            if let Some(local) = opts.transport
                && local != peer_kind
            {
                return Err(SessionError::TransportMismatch {
                    local,
                    peer: peer_kind,
                });
            }

            let mut transport = build_transport(opts, peer_kind, &observed, fingerprint)?;
            let my_endpoint = transport.prepare().await?;
            let my_blob = aead::seal(k, &aad, &wrap_endpoint(peer_kind, &my_endpoint))?;
            client::send_blob(ws, channel_id, &my_blob).await?;

            crate::log_info!(
                transport = %peer_kind,
                auto = opts.transport.is_none(),
                "endpoint exchanged, establishing transport"
            );
            Ok(transport.establish(role, &psk, peer_endpoint).await?)
        }
    }
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

    /// 发送端默认 WireGuard；接收端/裸 default 都是 auto。
    #[test]
    fn default_transport_is_wireguard_and_receiver_is_auto() {
        assert_eq!(
            SessionOptions::sender_default().transport,
            Some(TransportKind::WireGuard)
        );
        assert_eq!(SessionOptions::default().transport, None);
        assert_eq!(SessionOptions::receiver_default().transport, None);
        assert_eq!(
            SessionOptions::sender_default().congestion,
            crate::options::Congestion::Auto
        );
    }

    /// 端点头能正确往返。
    #[test]
    fn endpoint_header_roundtrip() {
        for kind in [TransportKind::WireGuard, TransportKind::Quic] {
            let payload = b"opaque-endpoint";
            let wrapped = wrap_endpoint(kind, payload);
            let (got_kind, got_payload) = parse_endpoint(&wrapped).unwrap();
            assert_eq!(got_kind, kind);
            assert_eq!(got_payload, payload);
        }
    }

    /// 端点头损坏/版本/类型非法 → 明确报错。
    #[test]
    fn bad_endpoint_header_errors() {
        assert!(matches!(
            parse_endpoint(&[]),
            Err(SessionError::BadEndpointHeader(_))
        ));
        assert!(matches!(
            parse_endpoint(&[b'X', b'X', 1, 2, b'p']),
            Err(SessionError::BadEndpointHeader(_))
        ));
        assert!(matches!(
            parse_endpoint(&[b'H', b'E', 2, 2, b'p']),
            Err(SessionError::BadEndpointHeader(_))
        ));
        assert!(matches!(
            parse_endpoint(&[b'H', b'E', 1, 0xff, b'p']),
            Err(SessionError::BadEndpointHeader(_))
        ));
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
        assert!(parse_hex32(&format!("中{}", "0".repeat(61))).is_none());
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

    /// 传输协议选择：CLI 字符串 → 枚举。
    #[test]
    fn transport_kind_roundtrip() {
        for (s, kind) in [
            ("wireguard", TransportKind::WireGuard),
            ("quic", TransportKind::Quic),
        ] {
            assert_eq!(TransportKind::parse(s), Some(kind));
            assert_eq!(kind.as_str(), s);
        }
    }
}
