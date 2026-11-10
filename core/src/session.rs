//! 会话编排：从**传输码**到「已建立、可 listen/dial 的传输连接」。
//!
//! 完整链路（DEVELOPMENT.md §3.2 步骤 1–7）：
//!
//! ```text
//! 传输码 → 连 server（SPKI pinning）→ allocate/join → wait_ready
//!   → CPace 得 K → derive_wg_psk(K, channel_id)
//!   → tx 选传输（当前统一 QUIC）→ prepare → 加端点头 → AEAD 交换 blob3
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
//! [3]     transport wire code（2=quic；1 是已移除的 WireGuard 旧码，对端需升级）
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
/// 端点头版本。（v1 = 含 WireGuard 的历史协议；v2 = QUIC-only 直连流载荷；
/// v3 = 多流切片数据面 + 链路档位，两端必须同代——旧端会得到明确的升级提示。）
const ENDPOINT_VERSION: u8 = 3;
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
    /// 端点头里的传输类型是本端不再提供的（包括已移除的 WireGuard wire 码 1，
    /// 意味着对端还是旧版本客户端）。
    #[error(
        "the peer announced a legacy transport (wire code {wire}); \
         the WireGuard data plane was removed — ask the peer to upgrade"
    )]
    LegacyPeerTransport { wire: u8 },
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
    /// 发送前预告；旧端忽略这个可选 JSON 扩展。
    pub offer: Option<crate::transfer::TransferOffer>,

    /// 当前协议的 MTU：QUIC UDP 载荷上限（自动 1452）。
    pub mtu: Option<usize>,
    /// UDP 绑定地址（默认 `[::]:0`）。
    pub udp_bind: Option<std::net::SocketAddr>,
    /// 传输协议：
    /// - tx：`Some(kind)` 指定；`None` 等同于当前默认（QUIC）。
    /// - rx：`None` = 自动跟随对端；`Some(kind)` = 强制并校验。
    pub transport: Option<TransportKind>,
    /// 统一拥塞控制；auto 在 QUIC 下为 BBR。
    pub congestion: crate::options::Congestion,
    /// 链路档位：数据面并发与拥塞启动参数（auto 按 RTT 在线决定）。
    pub profile: crate::options::LinkProfile,
    /// 发送端声明：载荷源可随机访问（能按切片 seek 发送，支持多流模式）。
    /// 随端点 bundle 公布给接收端；接收端据此决定是否用多流模式。
    pub multi_stream_capable: bool,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            offer: None,
            mtu: None,
            udp_bind: None,
            // 默认 auto：接收端跟随发送端；发送端用 sender_default()。
            transport: None,
            congestion: crate::options::Congestion::Auto,
            profile: crate::options::LinkProfile::Auto,
            multi_stream_capable: false,
        }
    }
}

impl SessionOptions {
    /// 发送端默认：QUIC（当前唯一数据面实现）。
    pub fn sender_default() -> Self {
        Self {
            transport: Some(TransportKind::Quic),
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
    /// 已建立的传输连接（当前为 QUIC）。
    pub conn: AnyConnection,
    /// 传输内 HTTP 的一次性 token（tx 用它拼 `GET /f/<token>`）。
    pub token: [u8; 16],
}

/// rx 会话（已建立，`dial` 后收文件）。
pub struct RxSession {
    /// 对端的加密预告；旧发送端为 None，后续从 HTTP 头补齐。
    pub offer: Option<crate::transfer::TransferOffer>,
    /// 已建立的传输连接（当前为 QUIC）。
    pub conn: AnyConnection,
    /// 传输内 HTTP 的一次性 token（rx 用同一个 token 拨号）。
    pub token: [u8; 16],
    /// 对端（发送方）声明载荷源可随机访问——多流切片模式的前提。
    pub peer_multi_stream: bool,
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
                "signaling connection timed out after 15s; check network connectivity and the server",
            ),
        ))
    })??;
    crate::log_info!("websocket connection established");
    Ok(ws)
}

/// tx：从传输码开始，建立到 rx 的传输连接（默认 QUIC）。
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
    opts.validate_for(opts.transport.unwrap_or(TransportKind::Quic))
        .map_err(SessionError::Options)?;
    if let Some(offer) = &opts.offer {
        offer.validate().map_err(SessionError::Options)?;
    }
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
    let (conn, _, _) = exchange_and_establish(
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
    let (conn, offer, peer_multi_stream) = exchange_and_establish(
        &mut ws,
        channel_id,
        &k,
        &token,
        observed,
        Role::Receiver,
        opts,
    )
    .await?;
    Ok(RxSession {
        conn,
        token,
        offer,
        peer_multi_stream,
    })
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
    transport.configure(opts.mtu, opts.congestion, opts.profile);
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
    match TransportKind::from_wire(bytes[3]) {
        Some(kind) => Ok((kind, &bytes[ENDPOINT_HEADER_LEN..])),
        // 线上码 1 曾属于已移除的 WireGuard —— 大概率对端还是旧版本客户端
        None => Err(SessionError::LegacyPeerTransport { wire: bytes[3] }),
    }
}

/// 两种端点当前都是 JSON 对象；旧解析器会忽略未知字段，HE v1 保持不变。
fn attach_offer(
    endpoint: &[u8],
    offer: Option<&crate::transfer::TransferOffer>,
) -> Result<Vec<u8>, SessionError> {
    let Some(offer) = offer else {
        return Ok(endpoint.to_vec());
    };
    offer.validate().map_err(SessionError::Options)?;
    let mut value: serde_json::Value = serde_json::from_slice(endpoint)
        .map_err(|e| SessionError::BadEndpointHeader(e.to_string()))?;
    value
        .as_object_mut()
        .ok_or_else(|| SessionError::BadEndpointHeader("expected endpoint object".into()))?
        .insert(
            "transfer_offer".into(),
            serde_json::to_value(offer)
                .map_err(|e| SessionError::BadEndpointHeader(e.to_string()))?,
        );
    serde_json::to_vec(&value).map_err(|e| SessionError::BadEndpointHeader(e.to_string()))
}

fn read_offer(endpoint: &[u8]) -> Result<Option<crate::transfer::TransferOffer>, SessionError> {
    let value: serde_json::Value = serde_json::from_slice(endpoint)
        .map_err(|e| SessionError::BadEndpointHeader(e.to_string()))?;
    let Some(offer) = value.get("transfer_offer") else {
        return Ok(None);
    };
    let offer: crate::transfer::TransferOffer = serde_json::from_value(offer.clone())
        .map_err(|e| SessionError::BadEndpointHeader(e.to_string()))?;
    offer.validate().map_err(SessionError::Options)?;
    Ok(Some(offer))
}

/// 交换加密端点字节，并 establish 传输。
///
/// tx 先 prepare+send，因为接收端需要先看到 tx 的端点头才能选传输实现。
/// 返回 (连接, 对端 offer, 对端多流能力)。
async fn exchange_and_establish(
    ws: &mut WsStream,
    channel_id: u16,
    k: &[u8; 32],
    token: &[u8; 16],
    observed: ObservedAddrs,
    role: Role,
    opts: &SessionOptions,
) -> Result<(AnyConnection, Option<crate::transfer::TransferOffer>, bool), SessionError> {
    let fingerprint = sha256_32(token);
    let psk = derive_wg_psk(k, channel_id);
    let aad = channel_id.to_be_bytes();

    match role {
        Role::Sender => {
            let kind = opts.transport.unwrap_or(TransportKind::Quic);
            let mut transport = build_transport(opts, kind, &observed, fingerprint)?;
            let my_endpoint = transport.prepare().await?;
            // 多流能力写进 QUIC 端点清单（类型安全字段，旧端忽略/拒绝由版本门负责）。
            let my_endpoint = if kind == TransportKind::Quic && opts.multi_stream_capable {
                let mut endpoint = crate::transport::quic::bundle::QuicEndpoint::decode(
                    &my_endpoint,
                )
                .map_err(|e| TransportError::Quic(crate::transport::QuicError::Bundle(e)))?;
                endpoint.multi_stream = true;
                endpoint
                    .encode()
                    .map_err(|e| TransportError::Quic(crate::transport::QuicError::Bundle(e)))?
            } else {
                my_endpoint
            };
            let my_endpoint = attach_offer(&my_endpoint, opts.offer.as_ref())?;
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
            Ok((
                transport.establish(role, &psk, peer_endpoint).await?,
                None,
                false,
            ))
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

            let offer = read_offer(peer_endpoint)?;
            // 对端多流能力：QUIC 端点清单里的显式字段；解析失败按不支持回退。
            let peer_multi_stream = match peer_kind {
                TransportKind::Quic => {
                    crate::transport::quic::bundle::QuicEndpoint::decode(peer_endpoint)
                        .map(|endpoint| endpoint.multi_stream)
                        .unwrap_or(false)
                }
            };
            let mut transport = build_transport(opts, peer_kind, &observed, fingerprint)?;
            let my_endpoint = transport.prepare().await?;
            let my_blob = aead::seal(k, &aad, &wrap_endpoint(peer_kind, &my_endpoint))?;
            client::send_blob(ws, channel_id, &my_blob).await?;

            crate::log_info!(
                transport = %peer_kind,
                auto = opts.transport.is_none(),
                peer_multi_stream,
                "endpoint exchanged, establishing transport"
            );
            Ok((
                transport.establish(role, &psk, peer_endpoint).await?,
                offer,
                peer_multi_stream,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optional_offer_is_compatible_and_validated() {
        let endpoint = br#"{"version":1,"udp_port":1234,"ipv6_candidates":[]}"#;
        assert_eq!(read_offer(endpoint).unwrap(), None);
        let offer = crate::transfer::TransferOffer {
            kind: crate::transfer::Kind::File,
            name: "empty.bin".into(),
            size: 0,
        };
        let extended = attach_offer(endpoint, Some(&offer)).unwrap();
        assert_eq!(read_offer(&extended).unwrap(), Some(offer));
        let malformed = br#"{"transfer_offer":{"kind":"file","name":"bad\nname","size":1}}"#;
        assert!(read_offer(malformed).is_err());
        let overflow =
            br#"{"transfer_offer":{"kind":"file","name":"file","size":18446744073709551615}}"#;
        assert!(read_offer(overflow).is_err());
    }

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

    /// 别的频道里截获的加密端点 blob 不能在本频道打开（AAD = 频道号大端）：
    /// 频道绑定 + 密钥通道双重核对，防止跨频道重放。走的是会话层的
    /// [`wrap_endpoint`]，和线上格式一致，而不是直接测裸 aead。
    #[test]
    fn endpoint_blob_is_channel_bound() {
        let payload = wrap_endpoint(TransportKind::Quic, br#"{"version":1,"udp_port":1}"#);
        let k = [0xab_u8; 32];
        let blob_channel_1 = crate::crypto::aead::seal(&k, &1u16.to_be_bytes(), &payload).unwrap();
        // 换频道打开 → 认证失败
        assert!(crate::crypto::aead::open(&k, &2u16.to_be_bytes(), &blob_channel_1).is_err());
        // 同频道不同密钥 → 认证失败（每个会话密钥唯一）
        assert!(
            crate::crypto::aead::open(&[0xcd_u8; 32], &1u16.to_be_bytes(), &blob_channel_1)
                .is_err()
        );
        // 篡改 blob 任何字节 → 认证失败
        let mut tampered = blob_channel_1.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 0x01;
        assert!(crate::crypto::aead::open(&k, &1u16.to_be_bytes(), &tampered).is_err());
    }

    /// 发送端默认 QUIC；接收端/裸 default 都是 auto。
    #[test]
    fn default_transport_is_quic_and_receiver_is_auto() {
        assert_eq!(
            SessionOptions::sender_default().transport,
            Some(TransportKind::Quic)
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
        let payload = b"opaque-endpoint";
        let wrapped = wrap_endpoint(TransportKind::Quic, payload);
        let (got_kind, got_payload) = parse_endpoint(&wrapped).unwrap();
        assert_eq!(got_kind, TransportKind::Quic);
        assert_eq!(got_payload, payload);
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
        // v3 + wire 2 是合法头（QUIC）；版本字节不对 / wire 码不对都要明确报错
        assert!(parse_endpoint(&[b'H', b'E', 3, 2, b'p']).is_ok());
        assert!(matches!(
            parse_endpoint(&[b'H', b'E', 2, 2, b'p']),
            Err(SessionError::BadEndpointHeader(h)) if h.contains("version")
        ));
        // 旧版 WireGuard wire 码 1 → 专门的 LegacyPeerTransport（提示对端升级）
        assert!(matches!(
            parse_endpoint(&[b'H', b'E', 3, 1, b'p']),
            Err(SessionError::LegacyPeerTransport { wire: 1 })
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

    /// 传输协议选择：CLI 字符串 → 枚举（当前仅 QUIC）。
    #[test]
    fn transport_kind_roundtrip() {
        assert_eq!(TransportKind::parse("quic"), Some(TransportKind::Quic));
        assert_eq!(TransportKind::Quic.as_str(), "quic");
        assert_eq!(TransportKind::parse("wireguard"), None);
    }
}
