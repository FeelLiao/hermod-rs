//! 传输层接口（契约）。
//!
//! 上层只依赖本模块的 [Transport] / [TransportConnection]；
//! 换传输协议 = 换实现，不动上层。当前有两个并列实现：
//!
//! - [crate::transport::quic]：QUIC + TLS 1.3 + BBR（默认）；
//! - [crate::transport::wireguard]：用户态 WireGuard + smoltcp TCP（可选回退）。
//!
//! 两个实现互不依赖，只共享本模块的 trait 与 [TransportKind]。
//!
//! ## 端点信息为什么是 opaque 字节
//!
//! 不同传输协议要公布的端点完全不同（WireGuard 公布公钥+UDP端口+IPv6候选，
//! QUIC 公布证书指纹+UDP端口+IPv6候选）。因此接口只把端点当作**传输层自定义的
//! opaque 字节**：上层负责搬运（AEAD 加密）与加密，但**不解析**。
//! 端点字节的格式、版本、凭证指纹校验都由各自实现负责。

use crate::crypto::cpace::Role;
use core::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};

/// 传输层字节流的组合 trait（trait object 只能包含一个非 auto trait）。
///
/// 需要把不同实现的流装箱成同一个类型；`Box<dyn AsyncRead + AsyncWrite>` 在
/// Rust 里不合法，所以用这个组合 trait 作为 `dyn` 的唯一非 auto trait。
pub trait TransportStream: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T> TransportStream for T where T: AsyncRead + AsyncWrite + Unpin + Send {}

/// 传输层网络统计快照（目前 QUIC 提供；WireGuard 返回 None）。
#[derive(Debug, Clone, Copy, Default)]
pub struct NetworkStats {
    /// 已发到 UDP 的字节数（含 QUIC 头、ACK、重传）。
    pub wire_bytes: u64,
    /// 已发 UDP 包数。
    pub sent_packets: u64,
    /// 丢失字节数。
    pub lost_bytes: u64,
    /// 丢失包数。
    pub lost_packets: u64,
    /// 平滑 RTT。
    pub rtt: Duration,
    /// 当前拥塞窗口。
    pub cwnd: u64,
    /// 当前路径 MTU。
    pub mtu: u16,
}

/// 可由后台任务更新、由进度显示读取的网络统计句柄。
#[derive(Clone, Default)]
pub struct NetworkStatsHandle(Arc<Mutex<NetworkStats>>);

impl NetworkStatsHandle {
    /// 新建全 0 快照。
    pub fn new() -> Self {
        Self::default()
    }

    /// 读取当前快照（短锁；锁中毒时返回全 0，不影响传输）。
    pub fn snapshot(&self) -> NetworkStats {
        self.0.lock().map(|s| *s).unwrap_or_default()
    }

    /// 由传输层后台任务更新。
    pub(crate) fn update(&self, stats: NetworkStats) {
        if let Ok(mut current) = self.0.lock() {
            *current = stats;
        }
    }
}

/// 数据面传输协议选择。
///
/// QUIC 是默认实现；WireGuard 仍可用 `--transport wireguard` 显式选择。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransportKind {
    /// 用户态 WireGuard + 进程内 TCP（可选回退）。
    WireGuard,
    /// QUIC（TLS 1.3 + 可插拔拥塞控制，默认 BBR）。
    #[default]
    Quic,
}

impl TransportKind {
    /// CLI/日志用的字符串值。
    pub fn as_str(self) -> &'static str {
        match self {
            TransportKind::WireGuard => "wireguard",
            TransportKind::Quic => "quic",
        }
    }

    /// 加密端点头里的线上编码（稳定值，不要随枚举顺序变化）。
    pub fn to_wire(self) -> u8 {
        match self {
            TransportKind::WireGuard => 1,
            TransportKind::Quic => 2,
        }
    }

    /// 从端点头解析线上编码。
    pub fn from_wire(value: u8) -> Option<Self> {
        match value {
            1 => Some(TransportKind::WireGuard),
            2 => Some(TransportKind::Quic),
            _ => None,
        }
    }

    /// 解析字符串；非法输入返回 `None`。
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "wireguard" | "wg" => Some(TransportKind::WireGuard),
            "quic" => Some(TransportKind::Quic),
            _ => None,
        }
    }
}

/// 标记「对端主动取消」的 io::Error payload。
///
/// 传输实现把 QUIC 的 cancel close code / RST 映射成这个 marker；HTTP 层用
/// [is_peer_cancelled] 识别，避免依赖错误字符串或平台相关的 ErrorKind。
#[derive(Debug)]
pub(crate) struct PeerCancelledMarker;

impl std::fmt::Display for PeerCancelledMarker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("peer cancelled the transfer")
    }
}

impl std::error::Error for PeerCancelledMarker {}

/// 构造带取消 marker 的 io::Error。
pub(crate) fn peer_cancelled_io() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::ConnectionReset, PeerCancelledMarker)
}

/// 判断一个 io::Error 是否由对端取消产生。
pub(crate) fn is_peer_cancelled(error: &std::io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|inner| inner.is::<PeerCancelledMarker>())
}

impl std::fmt::Display for TransportKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 传输层：负责把载荷字节在两端之间可靠地搬运。
///
/// 生命周期（对应 DEVELOPMENT.md §3.2 步骤 5–7）：
/// 1. [set_credential_fingerprint](Transport::set_credential_fingerprint)：会话层把
///    SHA-256(token) 交给传输层；
/// 2. [prepare](Transport::prepare)：准备本端端点，返回要公布给对端的字节；
/// 3. 上层把该字节用 CPace 密钥 AEAD 加密后经会合层交换；
/// 4. [establish](Transport::establish)：用对端端点 + PSK + 角色建立连接；
/// 5. 在返回的 [TransportConnection] 上跑载荷协议（tx `listen` / rx `dial`）。
pub trait Transport {
    /// 初始化/建连错误。
    type Error: std::error::Error + Send + Sync + 'static;

    /// 已建立的传输连接类型。
    type Connection: TransportConnection;

    /// 设置本会话的一次性凭证指纹（SHA-256(HTTP token)）。
    ///
    /// 实现必须把它写进 [prepare](Transport::prepare) 产出的端点字节，并在
    /// [establish](Transport::establish) 校验对端端点里的同一个值。
    fn set_credential_fingerprint(&mut self, fingerprint: [u8; 32]);

    /// 初始化传输层：生成会话语料（密钥、端口、本机地址等），
    /// 返回**本端要公布给对端的端点字节**（传输层自定义格式，上层不解析）。
    fn prepare(&mut self) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send;

    /// 用对端端点 + PSK + 角色建立传输连接。
    ///
    /// - `role`：`Sender`（tx，建立后负责 [TransportConnection::listen]）
    ///   或 `Receiver`（rx，负责 [TransportConnection::dial]）；
    /// - `psk`：由 CPace 的 K 派生（见 [crate::crypto::kdf::derive_wg_psk]），
    ///   两端各自本地派生，**从不过网**；QUIC 实现可以忽略它（TLS 证书已认证）；
    /// - `peer_endpoint`：对端 [Transport::prepare] 的返回值（已 AEAD 解密）。
    fn establish(
        &mut self,
        role: Role,
        psk: &[u8; 32],
        peer_endpoint: &[u8],
    ) -> impl Future<Output = Result<Self::Connection, Self::Error>> + Send;
}

/// 已建立的传输连接：在传输层内做双向字节流收发。
///
/// 载荷协议（传输内 HTTP/1.1）直接跑在 [listen](TransportConnection::listen) /
/// [dial](TransportConnection::dial) 返回的字节流上，不感知底层是 QUIC 还是
/// WireGuard。
pub trait TransportConnection {
    /// 连接层错误。
    type Error: std::error::Error + Send + Sync + 'static;

    /// 传输内字节流（可直接喂给 HTTP 的 AsyncRead/AsyncWrite）。
    type Stream: AsyncRead + AsyncWrite + Unpin + Send;

    /// 本端公布的 HTTP 端口（WireGuard 用它监听；QUIC 无端口，返回 0）。
    fn http_port(&self) -> u16;

    /// 对端公布的 HTTP 端口（WireGuard 用它拨号；QUIC 无端口，返回 0）。
    fn peer_http_port(&self) -> u16;

    /// tx：在传输内监听，等待对端连接并返回字节流。
    fn listen(&mut self) -> impl Future<Output = Result<Self::Stream, Self::Error>> + Send;

    /// rx：向传输内的对端拨号并返回字节流。
    fn dial(&mut self) -> impl Future<Output = Result<Self::Stream, Self::Error>> + Send;

    /// 取得一个可 clone 的中止句柄（Ctrl+C 取消时立即通知/关闭对端）。
    fn abort_handle(&self) -> AbortHandle;

    /// 优雅关闭：把已写入的数据真正送达对端后再返回。
    ///
    /// `orchestrate::*` 返回后应调用本方法，否则进程退出可能丢掉最后的裁决/尾数据。
    fn shutdown(&mut self) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// 可选的网络层统计（QUIC 提供；WireGuard 暂不提供）。
    ///
    /// 用于发送端显示实际 UDP 发送速率，而不是本地写入进度。
    fn network_stats(&self) -> Option<NetworkStatsHandle> {
        None
    }
}

/// 与具体传输协议无关的中止句柄。
///
/// 传输实现把「立即中止」的动作包成闭包；上层（CLI 的 Ctrl+C 处理器）只持有
/// 这个句柄，不需要知道底层是 WireGuard 的 RST 还是 QUIC 的 CONNECTION_CLOSE。
#[derive(Clone)]
pub struct AbortHandle(Arc<dyn Fn() + Send + Sync>);

impl AbortHandle {
    /// 用任意 `Fn()` 构造（通常是 `try_send(Abort)` 或 `endpoint.close(...)`）。
    pub fn new<F>(abort: F) -> Self
    where
        F: Fn() + Send + Sync + 'static,
    {
        Self(Arc::new(abort))
    }

    /// 触发中止。重复调用必须安全（底层实现自己保证幂等/忽略已关闭）。
    pub fn abort(&self) {
        (self.0)();
    }
}

impl std::fmt::Debug for AbortHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AbortHandle(..)")
    }
}

#[cfg(test)]
mod tests {
    //! 接口本身没有逻辑可测；这里用一个最小 mock 证明 trait 形状可用。

    use super::*;

    struct MockTransport;

    struct MockConnection;

    struct MockStream;

    impl AsyncRead for MockStream {
        fn poll_read(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            _buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Pending
        }
    }

    impl AsyncWrite for MockStream {
        fn poll_write(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            _buf: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            std::task::Poll::Pending
        }
        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    impl TransportConnection for MockConnection {
        type Error = std::io::Error;
        type Stream = MockStream;

        fn http_port(&self) -> u16 {
            1
        }
        fn peer_http_port(&self) -> u16 {
            2
        }
        async fn listen(&mut self) -> Result<Self::Stream, Self::Error> {
            Ok(MockStream)
        }
        async fn dial(&mut self) -> Result<Self::Stream, Self::Error> {
            Ok(MockStream)
        }
        fn abort_handle(&self) -> AbortHandle {
            AbortHandle::new(|| {})
        }
        async fn shutdown(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    impl Transport for MockTransport {
        type Error = std::io::Error;
        type Connection = MockConnection;

        fn set_credential_fingerprint(&mut self, _fingerprint: [u8; 32]) {}

        async fn prepare(&mut self) -> Result<Vec<u8>, Self::Error> {
            Ok(vec![1, 2, 3])
        }
        async fn establish(
            &mut self,
            _role: Role,
            _psk: &[u8; 32],
            _peer_endpoint: &[u8],
        ) -> Result<Self::Connection, Self::Error> {
            Ok(MockConnection)
        }
    }

    /// 用泛型函数消费 trait，证明接口可以泛型注入（换实现 = 换类型参数）。
    async fn run<C: Transport>(mut t: C) -> Result<(u16, u16), C::Error> {
        t.set_credential_fingerprint([0x11; 32]);
        let local = t.prepare().await?;
        let mut conn = t.establish(Role::Sender, &[0u8; 32], &local).await?;
        let _ = conn.listen().await.expect("mock listen");
        let _ = conn.abort_handle();
        conn.shutdown().await.expect("mock shutdown");
        Ok((conn.http_port(), conn.peer_http_port()))
    }

    #[tokio::test]
    async fn trait_shape_is_usable() {
        let (lp, pp) = run(MockTransport).await.unwrap();
        assert_eq!((lp, pp), (1, 2));
    }

    #[test]
    fn default_transport_is_quic() {
        assert_eq!(TransportKind::default(), TransportKind::Quic);
        assert_eq!(TransportKind::parse("quic"), Some(TransportKind::Quic));
        assert_eq!(
            TransportKind::parse("wireguard"),
            Some(TransportKind::WireGuard)
        );
        assert_eq!(TransportKind::parse("nope"), None);
        for kind in [TransportKind::WireGuard, TransportKind::Quic] {
            assert_eq!(TransportKind::from_wire(kind.to_wire()), Some(kind));
        }
        assert_eq!(TransportKind::from_wire(0), None);
        assert_eq!(TransportKind::from_wire(3), None);
    }

    #[test]
    fn abort_handle_is_protocol_agnostic() {
        let hit = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let h = AbortHandle::new({
            let hit = hit.clone();
            move || {
                hit.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        });
        h.abort();
        h.abort();
        assert_eq!(hit.load(std::sync::atomic::Ordering::SeqCst), 2);
    }
}
