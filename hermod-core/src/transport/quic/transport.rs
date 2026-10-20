//! QUIC 传输实现（默认数据面）。
//!
//! ## 为什么用 QUIC
//!
//! 旧数据面在 WireGuard 里跑 smoltcp TCP；丢包时 CUBIC 会把窗口打塌，重传还要
//! 等 RTO。QUIC 自带 RFC 9002 的 ACK 驱动丢包检测 + PTO、包级 pacing、PMTUD
//! 和可插拔拥塞控制（这里默认 BBR），在高 BDP/有丢包的链路上更不容易掉速。
//!
//! 连接方向：tx（Sender）是 QUIC client、主动出站；rx（Receiver）是 QUIC server、
//! 监听。载荷 HTTP/1.1 放在**一条 QUIC 双向流**上：tx 侧 `accept_bi`，rx 侧
//! `open_bi`（QUIC 允许 server 发起双向流），与
//! [TransportConnection::listen] / [TransportConnection::dial] 的语义一一对应。
//! 流开头先传 32 字节 session 指纹（SHA-256(token)）；server 只把匹配的流交给
//! HTTP，未授权连接无法占住唯一的 listen 流。
//!
//! ## 独立性
//!
//! 本模块不引用 `transport::wireguard` 的任何类型：endpoint 格式见
//! [crate::transport::quic::bundle]，IPv6 候选见 [crate::transport::quic::addr]，
//! 证书/握手见 [crate::transport::quic::auth]。

use crate::crypto::cpace::Role;
use crate::transport::iface::{AbortHandle, Transport, TransportConnection, peer_cancelled_io};
use crate::transport::quic::addr;
use crate::transport::quic::auth::{self, Identity};
use crate::transport::quic::bundle::{QUIC_ENDPOINT_VERSION, QuicEndpoint};
use futures_util::stream::{FuturesUnordered, StreamExt};
use quinn::VarInt;
use socket2::{Domain, Socket, Type};
use std::future::Future;
use std::io;
use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use subtle::ConstantTimeEq;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::oneshot;

/// 数据面连接建立超时（并行尝试全部候选地址的总预算）。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// 空闲超时；与 CLI 的 30 s 看门狗对齐。
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// 等待合法 client 发送 session 指纹前导的超时；防止未授权连接占住 listen。
const PREAMBLE_TIMEOUT: Duration = Duration::from_secs(5);
/// session 指纹前导长度（SHA-256(token)）。
const PREAMBLE_LEN: usize = 32;
/// QUIC 正常完成关闭码。
const CLOSE_DONE: u32 = 0;
/// QUIC 主动取消关闭码；对端据此显示 "peer cancelled the transfer"。
const CLOSE_CANCEL: u32 = 1;
/// 保活间隔。
const KEEPALIVE: Duration = Duration::from_secs(10);
/// 初始 RTT（比 quinn 默认 333 ms 更贴近常见网络）。
const INITIAL_RTT: Duration = Duration::from_millis(100);
/// 单条流的接收窗口 / 连接总接收窗口 / 发送窗口。
const IO_WINDOW: u64 = 64 * 1024 * 1024;
/// UDP socket 收发缓冲（内核可能截断）。
const SOCKET_BUF: usize = 8 * 1024 * 1024;
/// QUIC 初始 UDP payload 下限（RFC 9000 要求至少 1200）。
const DEFAULT_MTU: u16 = 1200;
/// 以太网内常见 IPv6 UDP payload 上限（1500 - 40 - 8）。
const MAX_MTU: u16 = 1452;

/// QUIC 拥塞控制算法。
///
/// BBR 不把丢包当作拥塞信号，而是估计瓶颈带宽和最小 RTT；在有随机丢包、
/// 浅缓冲的链路上通常比 CUBIC 更能跑满带宽。CUBIC/NewReno 保留作对照。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CongestionControl {
    /// BBR（默认）。
    #[default]
    Bbr,
    /// CUBIC。
    Cubic,
    /// NewReno（最保守）。
    NewReno,
}

impl CongestionControl {
    /// 字符串值。
    pub fn as_str(self) -> &'static str {
        match self {
            CongestionControl::Bbr => "bbr",
            CongestionControl::Cubic => "cubic",
            CongestionControl::NewReno => "newreno",
        }
    }

    /// 解析字符串；非法输入返回 `None`。
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "bbr" => Some(CongestionControl::Bbr),
            "cubic" => Some(CongestionControl::Cubic),
            "newreno" | "new_reno" => Some(CongestionControl::NewReno),
            _ => None,
        }
    }
}

impl std::fmt::Display for CongestionControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// QUIC 传输错误。
#[derive(Debug, thiserror::Error)]
pub enum QuicError {
    #[error("udp/io: {0}")]
    Io(#[from] io::Error),
    #[error("peer endpoint: {0}")]
    Bundle(#[from] serde_json::Error),
    #[error("peer endpoint version mismatch: got {got}, expected {expected}")]
    VersionMismatch { got: u8, expected: u8 },
    #[error("peer credential fingerprint does not match this session token")]
    CredentialMismatch,
    #[error("certificate generation: {0}")]
    Certificate(#[from] rcgen::Error),
    #[error("tls: {0}")]
    Tls(#[from] rustls::Error),
    #[error("quic config: {0}")]
    Config(String),
    #[error("quic handshake: {0}")]
    Handshake(String),
    #[error("quic handshake timed out after {0:?}")]
    HandshakeTimeout(Duration),
    #[error("quic connection: {0}")]
    Connection(String),
    #[error("transport not prepared")]
    NotPrepared,
    #[error("quic driver closed")]
    Closed,
}

/// QUIC 传输：prepare 后持有 UDP socket / 临时身份，establish 后交给驱动任务。
pub struct QuicTransport {
    socket: Option<std::net::UdpSocket>,
    identity: Option<Identity>,
    credential_fingerprint: [u8; 32],
    bind: Option<SocketAddrV6>,
    server_hint: Option<Ipv6Addr>,
    congestion: CongestionControl,
    initial_mtu: u16,
}

impl QuicTransport {
    /// 新建（尚未 prepare）。
    pub fn new() -> Self {
        Self {
            socket: None,
            identity: None,
            credential_fingerprint: [0u8; 32],
            bind: None,
            server_hint: None,
            congestion: CongestionControl::Bbr,
            initial_mtu: DEFAULT_MTU,
        }
    }

    /// 设置 UDP 绑定地址（`--listen`；默认 `[::]:0`）。
    pub fn set_udp_bind(&mut self, addr: Option<SocketAddrV6>) {
        self.bind = addr;
    }

    /// 注入信令服务器观测到的本端出口 IPv6 地址（仅作候选提示）。
    pub fn set_server_hint(&mut self, hint: Option<Ipv6Addr>) {
        self.server_hint = hint;
    }

    /// 设置拥塞控制算法。
    pub fn set_congestion(&mut self, congestion: CongestionControl) {
        self.congestion = congestion;
    }

    /// 设置初始 QUIC UDP payload（会被夹到 1200..=1452）。
    pub fn set_initial_mtu(&mut self, mtu: u16) {
        self.initial_mtu = mtu.clamp(DEFAULT_MTU, MAX_MTU);
    }
}

impl Default for QuicTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl Transport for QuicTransport {
    type Error = QuicError;
    type Connection = QuicConnection;

    fn set_credential_fingerprint(&mut self, fingerprint: [u8; 32]) {
        self.credential_fingerprint = fingerprint;
    }

    async fn prepare(&mut self) -> Result<Vec<u8>, QuicError> {
        let bind = self
            .bind
            .unwrap_or_else(|| SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0));
        let socket = bind_udp(bind)?;
        let udp_port = socket.local_addr()?.port();
        let identity = Identity::generate()?;

        // 候选顺序：显式绑定地址（测试/指定网卡）→ server 观测地址 → 本地枚举。
        let mut candidates = addr::enumerate_local_ipv6();
        if !bind.ip().is_unspecified() {
            let ip = *bind.ip();
            candidates.retain(|a| *a != ip);
            candidates.insert(0, ip);
        }
        if let Some(hint) = self.server_hint
            && addr::classify(&hint).is_some()
        {
            candidates.retain(|a| *a != hint);
            candidates.insert(0, hint);
        }

        let bundle = QuicEndpoint {
            version: QUIC_ENDPOINT_VERSION,
            udp_port,
            ipv6_candidates: candidates.clone(),
            cert_fingerprint: identity.fingerprint,
            credential_fingerprint: self.credential_fingerprint,
        };

        crate::log_info!(
            udp_port,
            congestion = %self.congestion,
            initial_mtu = self.initial_mtu,
            candidates = ?candidates,
            cert_fingerprint = %identity.fingerprint_hex(),
            "QUIC transport prepared"
        );

        self.socket = Some(socket);
        self.identity = Some(identity);
        Ok(bundle.encode()?)
    }

    async fn establish(
        &mut self,
        role: Role,
        _psk: &[u8; 32],
        peer_endpoint: &[u8],
    ) -> Result<QuicConnection, QuicError> {
        let peer = QuicEndpoint::decode(peer_endpoint)?;
        if peer.version != QUIC_ENDPOINT_VERSION {
            return Err(QuicError::VersionMismatch {
                got: peer.version,
                expected: QUIC_ENDPOINT_VERSION,
            });
        }
        if peer.credential_fingerprint != self.credential_fingerprint {
            return Err(QuicError::CredentialMismatch);
        }

        let socket = self.socket.take().ok_or(QuicError::NotPrepared)?;
        let identity = self.identity.take().ok_or(QuicError::NotPrepared)?;
        let peer_fingerprint = peer.cert_fingerprint;
        let credential_fingerprint = self.credential_fingerprint;
        let peer_addrs: Vec<SocketAddrV6> = peer
            .ipv6_candidates
            .iter()
            .map(|ip| SocketAddrV6::new(*ip, peer.udp_port, 0, 0))
            .collect();
        if peer_addrs.is_empty() {
            return Err(QuicError::Handshake(
                "peer advertised no IPv6 candidates".into(),
            ));
        }

        let transport = transport_config(self.congestion, self.initial_mtu);
        let runtime: Arc<dyn quinn::Runtime> = Arc::new(quinn::TokioRuntime);
        crate::log_info!(
            role = ?role,
            peer_udp_port = peer.udp_port,
            peer_candidates = ?peer.ipv6_candidates,
            congestion = %self.congestion,
            "establishing QUIC transport"
        );

        // 角色映射：
        // - tx（Sender）= QUIC client，主动向 rx 的所有候选发 Initial，和 WireGuard
        //   一样由发送端先出站，避免发送端防火墙丢弃首个入站 UDP；
        // - rx（Receiver）= QUIC server，只监听。
        // 但 HTTP 流的发起方向仍与 trait 语义一致：tx listen = accept_bi（接受
        // rx 打开的流），rx dial = open_bi（打开流）。QUIC 双向流允许 server 发起。
        let endpoint = match role {
            Role::Sender => {
                // 并行向 rx 的所有候选发 Initial（同时开 tx 侧防火墙针孔）。
                let tls = auth::client_config(peer_fingerprint)?;
                let crypto = quinn::crypto::rustls::QuicClientConfig::try_from(tls)
                    .map_err(|e| QuicError::Config(e.to_string()))?;
                let mut client = quinn::ClientConfig::new(Arc::new(crypto));
                client.transport_config(transport);
                let endpoint =
                    quinn::Endpoint::new(quinn::EndpointConfig::default(), None, socket, runtime)?;
                let (tx, rx) = oneshot::channel();
                tokio::spawn(connect_task(endpoint.clone(), client, peer_addrs, tx));
                QuicConnection::new(endpoint, rx, credential_fingerprint)
            }
            Role::Receiver => {
                // rx 是 server；先向 tx 的候选发几个 dummy UDP 包，打开 rx 侧
                // 有状态防火墙的返回路径（WireGuard 的「双方同时发」等价物）。
                for _ in 0..3 {
                    for addr in &peer_addrs {
                        let _ = socket.send_to(b"hermod-quic-pinhole", addr);
                    }
                }
                let tls = auth::server_config(&identity)?;
                let crypto = quinn::crypto::rustls::QuicServerConfig::try_from(tls)
                    .map_err(|e| QuicError::Config(e.to_string()))?;
                let mut server = quinn::ServerConfig::with_crypto(Arc::new(crypto));
                server.transport_config(transport);
                let endpoint = quinn::Endpoint::new(
                    quinn::EndpointConfig::default(),
                    Some(server),
                    socket,
                    runtime,
                )?;
                let (tx, rx) = oneshot::channel();
                tokio::spawn(accept_task(endpoint.clone(), tx));
                QuicConnection::new(endpoint, rx, credential_fingerprint)
            }
        };

        Ok(endpoint)
    }
}

/// 已建立的 QUIC 连接。握手在后台进行；`listen`/`dial` 等待它完成。
pub struct QuicConnection {
    endpoint: quinn::Endpoint,
    handshake: Option<oneshot::Receiver<Result<quinn::Connection, QuicError>>>,
    conn: Option<quinn::Connection>,
    /// 本会话的 SHA-256(token)；QUIC 流开头的 32 字节前导，用于在 server 侧
    /// 拒绝未通过 session 认证的连接（TLS 只认证 server）。
    credential_fingerprint: [u8; 32],
    closed: bool,
}

impl QuicConnection {
    fn new(
        endpoint: quinn::Endpoint,
        handshake: oneshot::Receiver<Result<quinn::Connection, QuicError>>,
        credential_fingerprint: [u8; 32],
    ) -> Self {
        Self {
            endpoint,
            handshake: Some(handshake),
            conn: None,
            credential_fingerprint,
            closed: false,
        }
    }

    async fn connection(&mut self) -> Result<quinn::Connection, QuicError> {
        if let Some(conn) = &self.conn {
            return Ok(conn.clone());
        }
        let rx = self.handshake.take().ok_or(QuicError::Closed)?;
        let conn = rx.await.map_err(|_| QuicError::Closed)??;
        self.conn = Some(conn.clone());
        Ok(conn)
    }
}

impl TransportConnection for QuicConnection {
    type Error = QuicError;
    type Stream = QuicStream;

    fn http_port(&self) -> u16 {
        0
    }

    fn peer_http_port(&self) -> u16 {
        0
    }

    async fn listen(&mut self) -> Result<Self::Stream, QuicError> {
        let conn = self.connection().await?;
        loop {
            let (mut send, mut recv) = conn
                .accept_bi()
                .await
                .map_err(|e| QuicError::Connection(e.to_string()))?;
            let mut preamble = [0u8; PREAMBLE_LEN];
            let authenticated = match tokio::time::timeout(
                PREAMBLE_TIMEOUT,
                recv.read_exact(&mut preamble),
            )
            .await
            {
                Ok(Ok(())) => bool::from(preamble.ct_eq(&self.credential_fingerprint)),
                _ => false,
            };
            if authenticated {
                crate::log_info!("accepted authenticated QUIC bidirectional stream");
                return Ok(QuicStream::new(send, recv));
            }
            crate::log_warn!("rejected QUIC stream with missing/invalid session preamble");
            let _ = send.reset(VarInt::from_u32(0));
        }
    }

    async fn dial(&mut self) -> Result<Self::Stream, QuicError> {
        let conn = self.connection().await?;
        let (mut send, recv) = conn
            .open_bi()
            .await
            .map_err(|e| QuicError::Connection(e.to_string()))?;
        // 先发 32 字节 session 指纹；server 只把这个前缀匹配的流交给 HTTP。
        send.write_all(&self.credential_fingerprint)
            .await
            .map_err(|e| QuicError::Connection(e.to_string()))?;
        crate::log_info!("opened authenticated QUIC bidirectional stream");
        Ok(QuicStream::new(send, recv))
    }

    fn abort_handle(&self) -> AbortHandle {
        let endpoint = self.endpoint.clone();
        AbortHandle::new(move || {
            endpoint.close(VarInt::from_u32(CLOSE_CANCEL), b"cancelled");
        })
    }

    async fn shutdown(&mut self) -> Result<(), QuicError> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        if let Some(conn) = self.conn.take() {
            // 流级 FIN 已经在 QuicStream::poll_shutdown 里等到 ACK；这里只收尾连接。
            conn.close(VarInt::from_u32(CLOSE_DONE), b"done");
        } else {
            // 握手尚未完成就关闭：直接让 endpoint 停下来。
            self.endpoint.close(VarInt::from_u32(CLOSE_DONE), b"done");
        }
        let _ = tokio::time::timeout(Duration::from_secs(1), self.endpoint.wait_idle()).await;
        Ok(())
    }
}

impl Drop for QuicConnection {
    fn drop(&mut self) {
        if !self.closed {
            if let Some(conn) = &self.conn {
                conn.close(VarInt::from_u32(CLOSE_CANCEL), b"cancelled");
            }
            self.endpoint
                .close(VarInt::from_u32(CLOSE_CANCEL), b"cancelled");
        }
    }
}

/// 一条 QUIC 双向流，拼成 tokio 的 `AsyncRead + AsyncWrite`。
///
/// `poll_shutdown` 会先 `finish()` 再等待 `stopped()` —— 即等到对端 ACK 完
/// 所有流数据。这样 `orchestrate` 最后的裁决（POST /done 的 200/409）不会被
/// 进程退出/连接关闭截断。
type StoppedFuture =
    Pin<Box<dyn Future<Output = Result<Option<VarInt>, quinn::StoppedError>> + Send>>;

pub struct QuicStream {
    send: quinn::SendStream,
    recv: quinn::RecvStream,
    stopped: Option<StoppedFuture>,
}

impl QuicStream {
    fn new(send: quinn::SendStream, recv: quinn::RecvStream) -> Self {
        Self {
            send,
            recv,
            stopped: None,
        }
    }
}

fn is_cancel_code(code: VarInt) -> bool {
    code == VarInt::from_u32(CLOSE_CANCEL)
}

fn connection_is_cancelled(error: &quinn::ConnectionError) -> bool {
    match error {
        quinn::ConnectionError::ApplicationClosed(close) => is_cancel_code(close.error_code),
        quinn::ConnectionError::Reset => true,
        _ => false,
    }
}

fn map_read_error(error: quinn::ReadError) -> io::Error {
    let cancelled = match &error {
        quinn::ReadError::Reset(code) => is_cancel_code(*code),
        quinn::ReadError::ConnectionLost(inner) => connection_is_cancelled(inner),
        _ => false,
    };
    if cancelled {
        peer_cancelled_io()
    } else {
        error.into()
    }
}

fn map_write_error(error: quinn::WriteError) -> io::Error {
    let cancelled = match &error {
        quinn::WriteError::Stopped(code) => is_cancel_code(*code),
        quinn::WriteError::ConnectionLost(inner) => connection_is_cancelled(inner),
        _ => false,
    };
    if cancelled {
        peer_cancelled_io()
    } else {
        error.into()
    }
}

fn stopped_is_cancelled(error: &quinn::StoppedError) -> bool {
    match error {
        quinn::StoppedError::ConnectionLost(inner) => connection_is_cancelled(inner),
        _ => false,
    }
}

impl AsyncRead for QuicStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut().recv.poll_read_buf(cx, buf) {
            Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
            Poll::Ready(Err(error)) => Poll::Ready(Err(map_read_error(error))),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl AsyncWrite for QuicStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match Pin::new(&mut self.get_mut().send).poll_write(cx, buf) {
            Poll::Ready(Ok(written)) => Poll::Ready(Ok(written)),
            Poll::Ready(Err(error)) => Poll::Ready(Err(map_write_error(error))),
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.stopped.is_none() {
            // 已经 finish 过（ClosedStream）也继续等 stopped；重复调用是幂等的。
            let _ = this.send.finish();
            this.stopped = Some(Box::pin(this.send.stopped()));
        }
        match this
            .stopped
            .as_mut()
            .expect("stopped future must exist")
            .as_mut()
            .poll(cx)
        {
            // None = 对端确认收完所有流数据；Some(1) = 对端主动取消。
            // 其他 reset/连接错误通常意味着上层已经因错误结束，shutdown 不再二次报错。
            Poll::Ready(Ok(Some(code))) if is_cancel_code(code) => {
                Poll::Ready(Err(peer_cancelled_io()))
            }
            Poll::Ready(Ok(_)) => Poll::Ready(Ok(())),
            Poll::Ready(Err(error)) if stopped_is_cancelled(&error) => {
                Poll::Ready(Err(peer_cancelled_io()))
            }
            Poll::Ready(Err(_)) => Poll::Ready(Ok(())),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// 并行向所有候选地址发起 QUIC 握手，第一个成功者胜出。
async fn connect_task(
    endpoint: quinn::Endpoint,
    config: quinn::ClientConfig,
    addrs: Vec<SocketAddrV6>,
    tx: oneshot::Sender<Result<quinn::Connection, QuicError>>,
) {
    let mut attempts = FuturesUnordered::new();
    for addr in addrs {
        match endpoint.connect_with(config.clone(), SocketAddr::V6(addr), auth::SERVER_NAME) {
            Ok(connecting) => attempts.push(async move { (addr, connecting.await) }),
            Err(e) => crate::log_warn!(addr = %addr, error = %e, "QUIC connect setup failed"),
        }
    }
    if attempts.is_empty() {
        let _ = tx.send(Err(QuicError::Handshake(
            "no usable IPv6 candidates".into(),
        )));
        return;
    }

    let outcome = tokio::time::timeout(CONNECT_TIMEOUT, async {
        while let Some((addr, result)) = attempts.next().await {
            match result {
                Ok(conn) => {
                    crate::log_info!(peer = %addr, rtt_ms = conn.rtt().as_millis(), "QUIC handshake complete");
                    return Ok(conn);
                }
                Err(e) => crate::log_warn!(peer = %addr, error = %e, "QUIC candidate failed"),
            }
        }
        Err(QuicError::Handshake("all IPv6 candidates failed".into()))
    })
    .await;

    let out = match outcome {
        Ok(result) => result,
        Err(_) => Err(QuicError::HandshakeTimeout(CONNECT_TIMEOUT)),
    };
    let _ = tx.send(out);
}

/// 接收第一个完成 QUIC/TLS 握手的连接。
async fn accept_task(
    endpoint: quinn::Endpoint,
    tx: oneshot::Sender<Result<quinn::Connection, QuicError>>,
) {
    loop {
        let Some(incoming) = endpoint.accept().await else {
            let _ = tx.send(Err(QuicError::Closed));
            return;
        };
        let peer = incoming.remote_address();
        let connecting = match incoming.accept() {
            Ok(connecting) => connecting,
            Err(e) => {
                crate::log_warn!(peer = %peer, error = %e, "QUIC incoming accept failed");
                continue;
            }
        };
        match connecting.await {
            Ok(conn) => {
                crate::log_info!(peer = %peer, rtt_ms = conn.rtt().as_millis(), "QUIC handshake complete");
                let _ = tx.send(Ok(conn));
                return;
            }
            Err(e) => crate::log_warn!(peer = %peer, error = %e, "QUIC incoming handshake failed"),
        }
    }
}

/// 构造高吞吐 QUIC transport 配置：BBR + 大窗口 + PMTUD + 低 ACK 延迟。
fn transport_config(
    congestion: CongestionControl,
    initial_mtu: u16,
) -> Arc<quinn::TransportConfig> {
    let mut config = quinn::TransportConfig::default();
    config.max_idle_timeout(Some(
        IDLE_TIMEOUT
            .try_into()
            .expect("30s idle timeout fits in QUIC VarInt"),
    ));
    config.stream_receive_window(VarInt::from_u32(IO_WINDOW as u32));
    config.receive_window(VarInt::from_u32(IO_WINDOW as u32));
    config.send_window(IO_WINDOW);
    config.keep_alive_interval(Some(KEEPALIVE));
    config.initial_rtt(INITIAL_RTT);
    config.min_mtu(DEFAULT_MTU);
    config.initial_mtu(initial_mtu.clamp(DEFAULT_MTU, MAX_MTU));
    config.mtu_discovery_config(Some(quinn::MtuDiscoveryConfig::default()));

    // ACK 频率扩展：更快反馈带宽/RTT 样本，代价是多一点 ACK 流量。
    let mut ack = quinn::AckFrequencyConfig::default();
    ack.max_ack_delay(Some(Duration::from_millis(5)));
    config.ack_frequency_config(Some(ack));

    match congestion {
        CongestionControl::Bbr => {
            config.congestion_controller_factory(Arc::new(quinn::congestion::BbrConfig::default()));
        }
        CongestionControl::Cubic => {
            config
                .congestion_controller_factory(Arc::new(quinn::congestion::CubicConfig::default()));
        }
        CongestionControl::NewReno => {
            config.congestion_controller_factory(Arc::new(
                quinn::congestion::NewRenoConfig::default(),
            ));
        }
    }
    Arc::new(config)
}

/// 绑定 IPv6-only UDP socket，并尽量放大内核收发缓冲。
fn bind_udp(addr: SocketAddrV6) -> io::Result<std::net::UdpSocket> {
    let socket = Socket::new(Domain::IPV6, Type::DGRAM, None)?;
    socket.set_only_v6(true)?;
    socket.set_nonblocking(true)?;
    // 默认 ~208 KiB 的 UDP 缓冲在高吞吐下会直接丢包，误导拥塞控制。
    let _ = socket.set_recv_buffer_size(SOCKET_BUF);
    let _ = socket.set_send_buffer_size(SOCKET_BUF);
    socket.bind(&SocketAddr::V6(addr).into())?;
    crate::log_info!(
        local = %addr,
        recv_buffer = ?socket.recv_buffer_size().ok(),
        send_buffer = ?socket.send_buffer_size().ok(),
        "QUIC UDP socket bound"
    );
    Ok(socket.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn loopback_transport() -> QuicTransport {
        let mut t = QuicTransport::new();
        t.set_udp_bind(Some(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 0, 0, 0)));
        t
    }

    /// prepare 产出的 endpoint 是 QUIC 类型，带证书指纹和可用端口。
    #[tokio::test]
    async fn prepare_produces_quic_endpoint() {
        let mut t = loopback_transport();
        t.set_credential_fingerprint([0x42; 32]);
        let bytes = t.prepare().await.unwrap();
        let b = QuicEndpoint::decode(&bytes).unwrap();
        assert_eq!(b.version, QUIC_ENDPOINT_VERSION);
        assert_eq!(b.credential_fingerprint, [0x42; 32]);
        assert_ne!(b.cert_fingerprint, [0u8; 32]);
        assert_ne!(b.udp_port, 0);
        assert!(b.ipv6_candidates.contains(&Ipv6Addr::LOCALHOST));
    }

    /// 两个 QUICTransport 在本机回环完成 TLS 握手 + 双向流传输。
    #[tokio::test]
    async fn end_to_end_stream() {
        let mut tx = loopback_transport();
        let mut rx = loopback_transport();
        tx.set_credential_fingerprint([0x11; 32]);
        rx.set_credential_fingerprint([0x11; 32]);
        let tx_ep = tx.prepare().await.unwrap();
        let rx_ep = rx.prepare().await.unwrap();
        let psk = crate::crypto::kdf::derive_wg_psk(&[0x77; 32], 9);

        let mut tx_conn = tx.establish(Role::Sender, &psk, &rx_ep).await.unwrap();
        let rx_conn = rx.establish(Role::Receiver, &psk, &tx_ep).await.unwrap();

        // QUIC 的 accept_bi 在收到第一条 STREAM 帧时才会返回；因此客户端必须在
        // listen 的同时就开始写，不能先 join(listen, dial) 再写。
        let data = vec![0x5a_u8; 2 * 1024 * 1024];
        let expected = data.clone();
        let client_task = tokio::spawn(async move {
            let mut client_conn = rx_conn;
            let mut client = client_conn.dial().await.unwrap();
            client.write_all(&data).await.unwrap();
            client.shutdown().await.unwrap();
            client_conn
        });

        let mut server = tokio::time::timeout(Duration::from_secs(20), tx_conn.listen())
            .await
            .expect("QUIC 握手/建流超时")
            .unwrap();
        let mut got = Vec::with_capacity(expected.len());
        tokio::time::timeout(Duration::from_secs(30), server.read_to_end(&mut got))
            .await
            .expect("QUIC 数据传输超时")
            .unwrap();
        assert_eq!(got.len(), expected.len());
        assert_eq!(got, expected);

        let mut rx_conn = client_task.await.unwrap();
        tx_conn.shutdown().await.unwrap();
        rx_conn.shutdown().await.unwrap();
    }

    /// 凭证指纹不一致必须在握手前拒绝。
    #[tokio::test]
    async fn credential_mismatch_is_rejected() {
        let mut tx = loopback_transport();
        let mut rx = loopback_transport();
        tx.set_credential_fingerprint([0x01; 32]);
        rx.set_credential_fingerprint([0x02; 32]);
        let _tx_ep = tx.prepare().await.unwrap();
        let rx_ep = rx.prepare().await.unwrap();
        let psk = crate::crypto::kdf::derive_wg_psk(&[0x88; 32], 3);
        let err = tx.establish(Role::Sender, &psk, &rx_ep).await.err();
        assert!(matches!(err, Some(QuicError::CredentialMismatch)));
    }

    /// 错误的对端证书指纹必须让握手失败（钉扎生效）。
    #[tokio::test]
    async fn wrong_certificate_fingerprint_fails_handshake() {
        let mut tx = loopback_transport();
        let mut rx = loopback_transport();
        tx.set_credential_fingerprint([0x33; 32]);
        rx.set_credential_fingerprint([0x33; 32]);
        let tx_ep = tx.prepare().await.unwrap();
        let mut rx_ep = QuicEndpoint::decode(&rx.prepare().await.unwrap()).unwrap();
        // 客户端只试 ::1，避免在测试里等不可达候选的超时。
        rx_ep.ipv6_candidates = vec![Ipv6Addr::LOCALHOST];
        // 把 rx（QUIC server）公布的证书指纹改坏；tx（client）必须拒绝它。
        rx_ep.cert_fingerprint = [0x00; 32];
        let rx_ep = rx_ep.encode().unwrap();
        let psk = crate::crypto::kdf::derive_wg_psk(&[0x99; 32], 4);

        let mut tx_conn = tx.establish(Role::Sender, &psk, &rx_ep).await.unwrap();
        let _rx_conn = rx.establish(Role::Receiver, &psk, &tx_ep).await.unwrap();
        let result = tokio::time::timeout(Duration::from_secs(10), tx_conn.listen()).await;
        assert!(
            matches!(result, Ok(Err(_))),
            "握手必须因为证书指纹不匹配而失败"
        );
    }

    /// 对端主动取消时，流上的 io error 必须带 peer-cancelled marker。
    #[tokio::test]
    async fn peer_cancel_maps_to_marker() {
        let mut tx = loopback_transport();
        let mut rx = loopback_transport();
        tx.set_credential_fingerprint([0x12; 32]);
        rx.set_credential_fingerprint([0x12; 32]);
        let tx_ep = tx.prepare().await.unwrap();
        let rx_ep = rx.prepare().await.unwrap();
        let psk = crate::crypto::kdf::derive_wg_psk(&[0x34; 32], 12);
        let mut tx_conn = tx.establish(Role::Sender, &psk, &rx_ep).await.unwrap();
        let mut rx_conn = rx.establish(Role::Receiver, &psk, &tx_ep).await.unwrap();

        let (server, client) = tokio::time::timeout(Duration::from_secs(20), async {
            tokio::join!(tx_conn.listen(), rx_conn.dial())
        })
        .await
        .expect("QUIC 建流超时");
        let _server = server.unwrap();
        let mut client = client.unwrap();

        tx_conn.abort_handle().abort();
        let mut buf = [0u8; 1];
        let error = tokio::time::timeout(Duration::from_secs(10), client.read(&mut buf))
            .await
            .expect("取消后读应返回")
            .expect_err("取消后读应报错");
        assert!(
            crate::transport::iface::is_peer_cancelled(&error),
            "取消错误必须带 peer-cancelled marker: {error}"
        );
    }

    /// 手动吞吐基准：`cargo test -p hermod-core --release -- --ignored quic_throughput`。
    #[tokio::test]
    #[ignore = "throughput benchmark; run manually in release mode"]
    async fn quic_throughput() {
        let mut tx = loopback_transport();
        let mut rx = loopback_transport();
        tx.set_credential_fingerprint([0x44; 32]);
        rx.set_credential_fingerprint([0x44; 32]);
        let tx_ep = tx.prepare().await.unwrap();
        let rx_ep = rx.prepare().await.unwrap();
        let psk = crate::crypto::kdf::derive_wg_psk(&[0xaa; 32], 1);
        let mut tx_conn = tx.establish(Role::Sender, &psk, &rx_ep).await.unwrap();
        let rx_conn = rx.establish(Role::Receiver, &psk, &tx_ep).await.unwrap();

        const TOTAL: usize = 256 * 1024 * 1024;
        let data = vec![0xa5_u8; 1024 * 1024];
        let start = std::time::Instant::now();
        let client_task = tokio::spawn(async move {
            let mut client_conn = rx_conn;
            let mut client = client_conn.dial().await.unwrap();
            for _ in 0..(TOTAL / data.len()) {
                client.write_all(&data).await.unwrap();
            }
            client.shutdown().await.unwrap();
            client_conn
        });

        let mut server = tx_conn.listen().await.unwrap();
        let mut buf = vec![0u8; 1024 * 1024];
        let mut total = 0usize;
        loop {
            let n = server.read(&mut buf).await.unwrap();
            if n == 0 {
                break;
            }
            total += n;
        }
        let mut rx_conn = client_task.await.unwrap();
        let secs = start.elapsed().as_secs_f64();
        println!(
            "QUIC throughput: {:.1} MiB/s ({total} bytes in {secs:.3}s)",
            total as f64 / 1_048_576.0 / secs
        );
        assert_eq!(total, TOTAL);
        tx_conn.shutdown().await.unwrap();
        rx_conn.shutdown().await.unwrap();
    }
}
