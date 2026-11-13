//! 信令服务器（Rust 版，对齐 Go `internal/server`）：WebSocket 信令中继 +
//! `/cert` 证书下发 + 同端口 UDP 反射（见 [`reflector`]）。
//!
//! 文件数据**从不**经过服务器：它只中转加密的握手 blob（≤64 KiB/条、
//! 每频道有 blob 条数上限）、观测客户端源地址（allocate/join 的 ok 应答）
//! 并在 TLS 上服务 `/cert`（PEM，供 `hermod trust` 钉扎）。
//! 线上格式（[`crate::signal::protocol`]）与 Go server 逐字节兼容，
//! 两种实现可互换部署，客户端无感。
//!
//! 与 Go 的锁序对照：Go 在 `handleJoin`/`handleAllocate` 里**持锁写响应**，
//! 因为 ready → ok → 入列必须原子——否则 sender 的 relay 可能在 receiver
//! 入列前转发 blob（丢消息）。Rust 版让 `waiters`（tokio Mutex）跨这些
//! await 持锁达成同样的原子性；每个连接自身的并发写安全由 sink 的
//! tokio Mutex 保证。锁序恒为 waiters → sink，无环，不会死锁。

pub mod cert;
mod devices;
pub mod ratelimit;
pub mod reflector;
pub mod store;

use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

use crate::signal::protocol::{MAX_MESSAGE_SIZE, Message, MsgType};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message as Frame;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

pub use ratelimit::RateLimiter;
pub use store::{
    DEFAULT_MAX_BLOBS_PER_CHANNEL, DEFAULT_MAX_CHANNELS_PER_IP, DEFAULT_MAX_CPACE_FAILURES,
    MemoryStore,
};

/// 服务器行为参数（对应 Go serve 命令的旗标）。
#[derive(Debug, Clone)]
pub struct ServerLimits {
    /// 频道 TTL + 连接空闲读超时。
    pub ttl: Duration,
    /// 每频道中继 blob 上限。
    pub max_blobs_per_channel: usize,
    /// CPace 协议违规上限，超过即弃置频道。
    pub max_cpace_failures: usize,
}

/// TLS 流（已解密一侧）。
pub type TlsStream = tokio_rustls::server::TlsStream<tokio::net::TcpStream>;
/// 已升级的 WebSocket 流（升级前读进的请求头经 [`PrefixedStream`] 重放）。
pub type WsStream = tokio_tungstenite::WebSocketStream<PrefixedStream<TlsStream>>;

type WsSink = Arc<Mutex<futures_util::stream::SplitSink<WsStream, Frame>>>;
type WsSource = futures_util::stream::SplitStream<WsStream>;

/// 信令服务器。
pub struct Server {
    limits: ServerLimits,
    store: Arc<MemoryStore>,
    cert_rl: Arc<RateLimiter>,
    ws_rl: Arc<RateLimiter>,
    join_rl: Arc<RateLimiter>,
    /// `/cert` 端点响应体（PEM）；空串时端点返回 404（与 Go 一致）。
    cert_pem: String,
    next_conn_id: AtomicU64,
    waiters: Mutex<HashMap<u16, Vec<Waiter>>>,
    devices: devices::Devices,
}

#[derive(Clone)]
struct Waiter {
    sink: WsSink,
    is_sender: bool,
    conn_id: u64,
    device_call_token: Option<String>,
    channel_owner_conn_id: Option<u64>,
}

impl Waiter {
    async fn close(&self) {
        let _ = self.sink.lock().await.close().await;
    }

    async fn send(&self, msg: &Message) {
        let frame = match serde_json::to_string(msg) {
            Ok(json) => Frame::text(json),
            Err(_) => return,
        };
        let _ = self.sink.lock().await.send(frame).await;
    }
}

/// 由服务器证书构建 TLS acceptor（TLS 1.2/1.3，默认 provider；自签 + 客户端钉扎）。
pub fn tls_acceptor(cert: &cert::ServerCert) -> Result<tokio_rustls::TlsAcceptor, rustls::Error> {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    let config =
        rustls::ServerConfig::builder_with_provider(crate::transport::quic::auth::provider())
            .with_safe_default_protocol_versions()?
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(cert.cert_der.clone())],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(cert.key_der.clone())),
            )?;
    Ok(tokio_rustls::TlsAcceptor::from(Arc::new(config)))
}

impl Server {
    /// 新建服务器；`cert_pem` 为空时 `/cert` 返回 404（与 Go 一致）。
    pub fn new(
        limits: ServerLimits,
        store: Arc<MemoryStore>,
        rate_limit: f64,
        rate_burst: f64,
        cert_pem: String,
    ) -> Arc<Self> {
        Arc::new(Self {
            limits,
            store,
            cert_rl: Arc::new(RateLimiter::new(rate_limit, rate_burst)),
            ws_rl: Arc::new(RateLimiter::new(rate_limit, rate_burst)),
            join_rl: Arc::new(RateLimiter::new(rate_limit, rate_burst)),
            cert_pem,
            next_conn_id: AtomicU64::new(0),
            waiters: Mutex::new(HashMap::new()),
            devices: devices::Devices::default(),
        })
    }

    /// 接受循环 + 后台 GC（频道过期清理 / 限流桶清理）。
    pub async fn run(
        self: Arc<Self>,
        listener: TcpListener,
        acceptor: tokio_rustls::TlsAcceptor,
        reflector: Option<reflector::ReflectorHandle>,
    ) -> io::Result<()> {
        let this = self.clone();
        let gc = tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(60));
            let mut round = 0_u64;
            loop {
                tick.tick().await;
                round += 1;
                let expired = this.store.purge_expired_owned();
                if !expired.is_empty() {
                    this.purge_expired_waiters(expired).await;
                }
                if round.is_multiple_of(10) {
                    let stale = Duration::from_secs(30 * 60);
                    this.cert_rl.cleanup(stale);
                    this.ws_rl.cleanup(stale);
                    this.join_rl.cleanup(stale);
                }
            }
        });

        let result = self.accept_loop(listener, acceptor).await;
        gc.abort();
        drop(reflector);
        result
    }

    async fn accept_loop(
        self: &Arc<Self>,
        listener: TcpListener,
        acceptor: tokio_rustls::TlsAcceptor,
    ) -> io::Result<()> {
        loop {
            let (tcp, _) = listener.accept().await?;
            let this = self.clone();
            tokio::spawn(this.serve_conn(tcp, acceptor.clone()));
        }
    }

    async fn serve_conn(
        self: Arc<Self>,
        tcp: tokio::net::TcpStream,
        acceptor: tokio_rustls::TlsAcceptor,
    ) {
        let remote = match tcp.peer_addr() {
            Ok(addr) => addr,
            Err(_) => return,
        };
        crate::log_debug!(remote = %remote, "TLS connection accepted");
        let mut tls =
            match tokio::time::timeout(Duration::from_secs(10), acceptor.accept(tcp)).await {
                Ok(Ok(tls)) => tls,
                _ => return,
            };
        // 读请求头（ReadHeaderTimeout=10s；16 KiB 上限防 Slowloris/超大头）。
        let head = match read_request_head(&mut tls).await {
            Ok(head) => head,
            Err(_) => {
                let _ = write_http_response(&mut tls, 431, b"request header too large", None).await;
                return;
            }
        };
        let mut io = PrefixedStream {
            prefix: head.raw,
            pos: 0,
            inner: tls,
        };
        match (head.method.as_str(), head.path.as_str()) {
            ("GET", "/ws" | "/device") => {
                // 浏览器来源的跨域 WS 一律拒绝（对应 Go CheckOrigin）；
                // CLI/GUI 客户端不带 Origin，不受影响。
                if head.has_origin {
                    crate::log_warn!(remote = %remote, "rejected WebSocket with Origin header");
                    let _ = write_http_response(
                        &mut io,
                        403,
                        b"cross-origin websocket not allowed",
                        None,
                    )
                    .await;
                    return;
                }
                if !self.ws_rl.allow(remote) {
                    crate::log_warn!(remote = %remote, "request rate-limited");
                    let _ = write_http_response(&mut io, 429, b"too many requests", None).await;
                    return;
                }
                let mut ws_config = WebSocketConfig::default();
                ws_config.max_message_size = Some(MAX_MESSAGE_SIZE);
                ws_config.max_frame_size = Some(MAX_MESSAGE_SIZE);
                // 对应 Go upgrader 的空 CheckOrigin 语义已在上面处理；升级保持 101 原样。
                let ws = match tokio_tungstenite::accept_hdr_async_with_config(
                    io,
                    NoopCallback,
                    Some(ws_config),
                )
                .await
                {
                    Ok(ws) => ws,
                    Err(error) => {
                        crate::log_warn!(remote = %remote, error = %error, "WebSocket upgrade failed");
                        return;
                    }
                };
                if head.path == "/device" {
                    let conn_id = self.next_conn_id.fetch_add(1, Ordering::Relaxed);
                    self.devices.serve(&self, ws, conn_id).await;
                } else {
                    self.serve_client(ws, remote).await;
                }
            }
            ("GET", "/cert") => {
                if !self.cert_rl.allow(remote) {
                    crate::log_warn!(remote = %remote, "cert endpoint rate-limited");
                    let _ = write_http_response(&mut io, 429, b"too many requests", None).await;
                    return;
                }
                if self.cert_pem.is_empty() {
                    let _ =
                        write_http_response(&mut io, 404, b"certificate not available", None).await;
                    return;
                }
                let _ = write_http_response(
                    &mut io,
                    200,
                    self.cert_pem.as_bytes(),
                    Some("application/x-pem-file"),
                )
                .await;
            }
            _ => {
                let _ = write_http_response(&mut io, 404, b"not found", None).await;
            }
        }
    }

    /// 读完首条消息后按角色分流；之后进入 blob 中继循环。
    async fn serve_client(&self, ws: WsStream, remote: SocketAddr) {
        let conn_id = self.next_conn_id.fetch_add(1, Ordering::Relaxed);
        let (sink, mut source) = ws.split();
        let sink: WsSink = Arc::new(Mutex::new(sink));
        // 首条消息（空闲读超时 = ttl；pong 保活的等价：客户端有数据就算活）。
        let first = match tokio::time::timeout(self.limits.ttl, source.next()).await {
            Ok(Some(Ok(frame))) => frame,
            _ => {
                crate::log_debug!(remote = %remote, "no first message before idle timeout");
                return;
            }
        };
        let msg = match frame_to_message(first) {
            Ok(msg) => msg,
            Err(_) => return,
        };
        crate::log_debug!(
            remote = %remote,
            msg_type = ?msg.msg_type,
            channel_id = msg.channel_id,
            "first message received",
        );
        match msg.msg_type {
            MsgType::Allocate => {
                self.handle_allocate(&sink, source, remote, msg.channel_id, conn_id)
                    .await
            }
            MsgType::Join => {
                self.handle_join(&sink, source, remote, msg.channel_id, conn_id)
                    .await
            }
            _ => {
                crate::log_warn!(remote = %remote, msg_type = ?msg.msg_type, "unknown first message type");
                send_error(&sink, "first message must be 'allocate' or 'join'").await;
            }
        }
        crate::log_debug!(remote = %remote, "connection closed");
    }

    /// allocate：注册频道 → 清陈旧 waiter → ok（带观测地址）→ 有接收端在等则 ready
    /// → 进入中继。
    async fn handle_allocate(
        &self,
        sink: &WsSink,
        source: WsSource,
        remote: SocketAddr,
        channel_id: u16,
        conn_id: u64,
    ) {
        if let Err(error) =
            self.store
                .allocate_channel_owned(channel_id, self.limits.ttl, remote, conn_id)
        {
            crate::log_warn!(channel_id, remote = %remote, error = %error, "channel allocation failed");
            send_error(sink, "could not allocate channel").await;
            return;
        }
        let mut random = [0; 32];
        if getrandom::fill(&mut random).is_err() {
            self.store.delete_channel_if_owner(channel_id, conn_id);
            return;
        }
        let device_call_token = crate::signal::device::hex(&random);
        // 持锁完成：清陈旧 waiter → ok → ready → 入列（与 Go 相同的原子性）。
        let mut map = self.waiters.lock().await;
        let mut stale = Vec::new();
        map.entry(channel_id).or_default().retain(|waiter| {
            if waiter.conn_id == conn_id {
                true
            } else {
                stale.push(waiter.clone());
                false
            }
        });
        let mut response = public_ip_response(remote.ip());
        response["device_call_token"] = serde_json::Value::String(device_call_token.clone());
        let payload = serde_json::to_vec(&response).unwrap_or_default();
        send_msg(sink, &Message::ok(channel_id, payload)).await;
        let receiver_waiting = map
            .entry(channel_id)
            .or_default()
            .iter()
            .any(|waiter| !waiter.is_sender);
        if receiver_waiting {
            send_msg(sink, &Message::ready(channel_id)).await;
            crate::log_debug!(channel_id, "sent ready to sender (receiver joined early)");
        }
        map.entry(channel_id).or_default().push(Waiter {
            sink: sink.clone(),
            is_sender: true,
            conn_id,
            device_call_token: Some(device_call_token),
            channel_owner_conn_id: Some(conn_id),
        });
        drop(map);
        for waiter in stale {
            crate::log_warn!(channel_id, "removing stale waiter for channel");
            waiter.close().await;
        }
        crate::log_info!(channel_id, remote = %remote, "channel allocated");
        self.relay(sink, source, channel_id, true, conn_id, conn_id)
            .await;
    }

    /// join：限流 → 频道必须存在 → 拒绝重复接收端 → 持锁完成 ready/ok/入列 → 中继。
    async fn handle_join(
        &self,
        sink: &WsSink,
        source: WsSource,
        remote: SocketAddr,
        channel_id: u16,
        conn_id: u64,
    ) {
        // join 按 IP 限流，拖慢频道枚举（对应 Go L-09）。
        if !self.join_rl.allow(remote) {
            crate::log_warn!(channel_id, remote = %remote, "join rate-limited");
            send_error(sink, "operation failed").await;
            return;
        }
        // 未分配的频道直接拒绝；错误文案刻意模糊，
        // 让客户端无法区分“频道不存在 / 重复接收端 / 暂时失败”。
        if !self.store.channel_exists(channel_id) {
            crate::log_warn!(channel_id, remote = %remote, "receiver attempted to join non-existent channel");
            send_error(sink, "operation failed").await;
            return;
        }

        let mut map = self.waiters.lock().await;
        let Some(channel_owner) = self.store.channel_owner(channel_id) else {
            drop(map);
            send_error(sink, "operation failed").await;
            return;
        };
        let entry = map.entry(channel_id).or_default();
        if entry.iter().any(|waiter| !waiter.is_sender) {
            crate::log_warn!(channel_id, remote = %remote, "duplicate receiver rejected");
            send_error(sink, "operation failed").await;
            return;
        }
        // 持锁顺序（对应 Go handleJoin 的注释）：ready 发给 sender → ok 发给自己
        // → 最后才把自己入列。这样 sender 的 relay 不会在 ok 之前转发 blob。
        let sender = entry.iter().find(|waiter| waiter.is_sender).cloned();
        if let Some(sender) = &sender {
            sender.send(&Message::ready(channel_id)).await;
        }
        let payload = serde_json::to_vec(&public_ip_response(remote.ip())).unwrap_or_default();
        send_msg(sink, &Message::ok(channel_id, payload)).await;
        entry.push(Waiter {
            sink: sink.clone(),
            is_sender: false,
            conn_id,
            device_call_token: None,
            channel_owner_conn_id: Some(channel_owner),
        });
        drop(map);

        crate::log_info!(channel_id, remote = %remote, "receiver joined channel");
        self.relay(sink, source, channel_id, false, conn_id, channel_owner)
            .await;
    }

    /// 中继循环：读本端的 blob 消息并转发给对端。
    async fn relay(
        &self,
        sink: &WsSink,
        mut source: WsSource,
        channel_id: u16,
        is_sender: bool,
        conn_id: u64,
        channel_owner: u64,
    ) {
        let role = if is_sender { "sender" } else { "receiver" };
        crate::log_debug!(channel_id, role, "relay loop started");
        let mut blob_count = 0_usize;
        loop {
            let frame = match tokio::time::timeout(self.limits.ttl, source.next()).await {
                Ok(Some(Ok(frame))) => frame,
                _ => {
                    crate::log_debug!(channel_id, role, "relay connection closed");
                    break;
                }
            };
            let msg = match frame_to_message(frame) {
                Ok(msg) => msg,
                Err(_) => break,
            };
            match msg.msg_type {
                MsgType::Blob => {
                    blob_count += 1;
                    if blob_count > self.limits.max_blobs_per_channel {
                        crate::log_warn!(
                            channel_id,
                            role,
                            count = blob_count,
                            "blob limit exceeded — closing connection"
                        );
                        send_error(sink, "blob limit exceeded").await;
                        break;
                    }
                    if let Err(error) = self.store.store_blob_owned(
                        channel_id,
                        channel_owner,
                        is_sender,
                        &msg.payload,
                    ) {
                        crate::log_warn!(channel_id, role, error = %error, "failed to store blob");
                        let message = if self
                            .record_failure_and_drop(channel_id, channel_owner)
                            .await
                        {
                            "handshake data store failed: channel terminated"
                        } else {
                            "handshake data store failed"
                        };
                        send_error(sink, message).await;
                        break;
                    }
                    let peer = {
                        let map = self.waiters.lock().await;
                        map.get(&channel_id)
                            .and_then(|waiters| {
                                waiters.iter().find(|waiter| {
                                    waiter.is_sender != is_sender
                                        && waiter.channel_owner_conn_id == Some(channel_owner)
                                })
                            })
                            .cloned()
                    };
                    match peer {
                        Some(peer) => peer.send(&msg).await,
                        None => {
                            crate::log_warn!(channel_id, role, "no peer available to receive blob");
                        }
                    }
                }
                _ => {
                    crate::log_warn!(channel_id, role, msg_type = ?msg.msg_type, "unexpected message type in relay");
                    self.record_failure_and_drop(channel_id, channel_owner)
                        .await;
                    send_error(sink, "unexpected message type").await;
                    break;
                }
            }
        }
        // 收尾：把自己从 waiters 摘掉；对端还挂着就关掉它（对端因此感知断连）。
        let peer = {
            let mut map = self.waiters.lock().await;
            let mut peer = None;
            if let Some(waiters) = map.get_mut(&channel_id) {
                let owns_lease = waiters.iter().any(|waiter| waiter.is_same_conn(sink));
                waiters.retain(|waiter| {
                    if waiter.is_same_conn(sink) {
                        false
                    } else {
                        if owns_lease && waiter.channel_owner_conn_id == Some(channel_owner) {
                            peer = Some(waiter.clone());
                        }
                        true
                    }
                });
                if waiters.is_empty() {
                    map.remove(&channel_id);
                }
            }
            // 删除与 waiter 摘除同属本次清理；旧代不能释放新分配的频道。
            if is_sender {
                self.store.delete_channel_if_owner(channel_id, conn_id);
            }
            peer
        };
        if let Some(peer) = peer {
            crate::log_debug!(
                channel_id,
                role,
                "closing peer — peer disconnected from relay"
            );
            peer.close().await;
        }
        crate::log_debug!(channel_id, role, "relay loop ended");
    }

    async fn purge_expired_waiters(&self, ids: Vec<(u16, Option<u64>)>) {
        let mut to_close = Vec::new();
        {
            let mut map = self.waiters.lock().await;
            for (id, owner) in ids {
                if let Some(waiters) = map.get_mut(&id) {
                    waiters.retain(|waiter| {
                        if waiter.channel_owner_conn_id == owner {
                            to_close.push(waiter.clone());
                            false
                        } else {
                            true
                        }
                    });
                    if waiters.is_empty() {
                        map.remove(&id);
                    }
                }
            }
        }
        for waiter in to_close {
            waiter.close().await;
        }
    }

    /// 弃置频道：给所有连接发最终错误并关闭，再从存储删除。可安全重复调用。
    async fn drop_channel_if_owner(&self, channel_id: u16, owner: u64) {
        let conns = {
            let mut map = self.waiters.lock().await;
            self.store.delete_channel_if_owner(channel_id, owner);
            let mut conns = Vec::new();
            if let Some(waiters) = map.get_mut(&channel_id) {
                waiters.retain(|waiter| {
                    if waiter.channel_owner_conn_id == Some(owner) {
                        conns.push(waiter.clone());
                        false
                    } else {
                        true
                    }
                });
                if waiters.is_empty() {
                    map.remove(&channel_id);
                }
            }
            conns
        };
        for waiter in &conns {
            waiter
                .send(&Message {
                    msg_type: MsgType::Error,
                    channel_id,
                    payload: Vec::new(),
                    error: "channel terminated: CPace failure limit exceeded".into(),
                })
                .await;
        }
        for waiter in &conns {
            waiter.close().await;
        }
    }

    /// 记一次协议失败；达到上限返回 true（频道已被弃置）。
    async fn record_failure_and_drop(&self, channel_id: u16, owner: u64) -> bool {
        match self.store.record_failure_owned(channel_id, owner) {
            Err(_) => true, // 频道可能已被删；按终态处理
            Ok(count) => {
                if count >= self.limits.max_cpace_failures {
                    crate::log_warn!(channel_id, "CPace failure limit reached — dropping channel");
                    self.drop_channel_if_owner(channel_id, owner).await;
                    true
                } else {
                    false
                }
            }
        }
    }
}

impl Waiter {
    fn is_same_conn(&self, sink: &WsSink) -> bool {
        Arc::ptr_eq(&self.sink, sink)
    }
}

/// 文本帧 → 信令消息；二进制帧/坏 JSON 都算协议错误（对应 gorilla ReadJSON 失败）。
fn frame_to_message(frame: Frame) -> Result<Message, ()> {
    match frame {
        Frame::Text(text) => serde_json::from_str(&text).map_err(|_| ()),
        _ => Err(()),
    }
}

async fn send_msg(sink: &WsSink, msg: &Message) {
    if let Ok(json) = serde_json::to_string(msg) {
        let _ = sink.lock().await.send(Frame::text(json)).await;
    }
}

async fn send_error(sink: &WsSink, message: &str) {
    send_msg(sink, &Message::error(message)).await;
}

/// allocate/join 应答的观测地址 JSON（对应 Go `publicIPResponse`）：
/// 带 `public_ip`，并按地址族补 `public_ipv4`/`public_ipv6`（解析失败按 IPv4 兜底，
/// 保证至少有一个族键存在）。
pub fn public_ip_response(ip: std::net::IpAddr) -> serde_json::Value {
    let mut response = serde_json::Map::new();
    response.insert(
        "public_ip".into(),
        serde_json::Value::String(ip.to_string()),
    );
    match ip {
        std::net::IpAddr::V4(_) => {
            response.insert(
                "public_ipv4".into(),
                serde_json::Value::String(ip.to_string()),
            );
        }
        std::net::IpAddr::V6(_) => {
            response.insert(
                "public_ipv6".into(),
                serde_json::Value::String(ip.to_string()),
            );
        }
    }
    serde_json::Value::Object(response)
}

/// 请求头（方法 / 路径 / 是否带 Origin）+ 原始字节（升级 WS 时重放）。
pub struct RequestHead {
    raw: Vec<u8>,
    method: String,
    path: String,
    has_origin: bool,
}

const HEAD_MAX: usize = 16 * 1024;

/// 读 HTTP/1.1 请求头直到 `\r\n\r\n`；原始字节保留在返回值里供 WS 升级重放。
async fn read_request_head(io: &mut TlsStream) -> io::Result<RequestHead> {
    let mut raw = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        if raw.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if raw.len() > HEAD_MAX {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "header too large",
            ));
        }
        let n = tokio::time::timeout(Duration::from_secs(10), io.read(&mut buf))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "read header timeout"))??;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "closed before header end",
            ));
        }
        raw.extend_from_slice(&buf[..n]);
    }
    parse_head(raw)
}

/// 解析请求头：请求行的方法/路径 + 是否带 Origin（大小写不敏感）。
fn parse_head(raw: Vec<u8>) -> io::Result<RequestHead> {
    let text = std::str::from_utf8(&raw)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "non-utf8 header"))?;
    let mut lines = text.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();
    let has_origin = lines.any(|line| {
        let (name, _) = line.split_once(':').unwrap_or(("", ""));
        name.trim().eq_ignore_ascii_case("origin")
    });
    Ok(RequestHead {
        raw,
        method,
        path,
        has_origin,
    })
}

/// 路由后的 TLS 流：先把升级前读进的请求头重放给解析器，再直通底层。
pub struct PrefixedStream<T> {
    prefix: Vec<u8>,
    pos: usize,
    inner: T,
}

impl<T: AsyncRead + Unpin> AsyncRead for PrefixedStream<T> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.pos < this.prefix.len() {
            let n = (this.prefix.len() - this.pos).min(buf.remaining());
            buf.put_slice(&this.prefix[this.pos..this.pos + n]);
            this.pos += n;
            return std::task::Poll::Ready(Ok(()));
        }
        std::pin::Pin::new(&mut this.inner).poll_read(cx, buf)
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for PrefixedStream<T> {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

/// 不修改升级应答的回调（用命名 struct 而不是闭包，绕开 FnOnce 推断限制）。
struct NoopCallback;

impl tokio_tungstenite::tungstenite::handshake::server::Callback for NoopCallback {
    fn on_request(
        self,
        _request: &tokio_tungstenite::tungstenite::handshake::server::Request,
        response: tokio_tungstenite::tungstenite::handshake::server::Response,
    ) -> Result<
        tokio_tungstenite::tungstenite::handshake::server::Response,
        tokio_tungstenite::tungstenite::handshake::server::ErrorResponse,
    > {
        Ok(response)
    }
}

/// 写一个最小 HTTP/1.1 响应（两个 GET 端点足够；无 body 压缩/分块）。
async fn write_http_response<W: AsyncWrite + Unpin>(
    io: &mut W,
    status: u16,
    body: &[u8],
    content_type: Option<&str>,
) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        _ => "Error",
    };
    let content_type = content_type.unwrap_or("text/plain; charset=utf-8");
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    io.write_all(head.as_bytes()).await?;
    io.write_all(body).await?;
    io.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn parse_head_extracts_method_path_and_origin() {
        let raw = b"GET /ws HTTP/1.1\r\nHost: h\r\nOrigin: https://evil.example\r\n\r\n";
        let head = parse_head(raw.to_vec()).unwrap();
        assert_eq!(head.method, "GET");
        assert_eq!(head.path, "/ws");
        assert!(head.has_origin);
        let raw = b"GET /ws HTTP/1.1\r\nX-Origin: x\r\n\r\n";
        let head = parse_head(raw.to_vec()).unwrap();
        assert!(!head.has_origin, "非 Origin 头不能误判");
        assert_eq!(head.path, "/ws");
    }

    #[test]
    fn observed_address_json_matches_go_and_client() {
        // 客户端 ObservedAddrs 只认 public_ipv4/public_ipv6 两个字段。
        let v4 = public_ip_response(Ipv4Addr::new(203, 0, 113, 9).into());
        assert_eq!(v4["public_ip"], "203.0.113.9");
        assert_eq!(v4["public_ipv4"], "203.0.113.9");
        assert!(v4.get("public_ipv6").is_none());
        let parsed: crate::signal::client::ObservedAddrs =
            serde_json::from_value(public_ip_response("2001:db8::1".parse().unwrap())).unwrap();
        assert_eq!(parsed.public_ipv6, "2001:db8::1");
    }
}
