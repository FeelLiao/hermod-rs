//! 传输层（data plane）：抽象「一条已认证的双向字节流」，与具体传输协议解耦。
//!
//! 当前有两个**并列、互不依赖**的实现：
//!
//! - [quic]：QUIC + TLS 1.3 + BBR（默认）；
//! - [wireguard]：用户态 WireGuard + smoltcp 进程内 TCP（可选回退）。
//!
//! 上层（[crate::session] / [crate::orchestrate]）只依赖 [iface::Transport] /
//! [iface::TransportConnection]；[AnyTransport] / [AnyConnection] 只是运行时分发，
//! QUIC 模块本身不引用 WireGuard 的任何类型。
//!
//! ## 目录
//!
//! ```text
//! transport/
//! ├── iface.rs      # trait + TransportKind + AbortHandle
//! ├── mod.rs        # AnyTransport / AnyConnection 运行时分发
//! ├── wireguard/    # 可选回退：WireGuard + smoltcp TCP
//! ├── http/         # 共享的传输内 HTTP/1.1
//! └── quic/         # 默认实现：QUIC endpoint / TLS 钉扎 / BBR
//! ```

pub mod http;
pub mod iface;
pub mod quic;
pub mod wireguard;

pub use iface::{
    AbortHandle, NetworkStats, NetworkStatsHandle, Transport, TransportConnection, TransportKind,
    TransportStream,
};
pub use quic::transport::{CongestionControl, QuicConnection, QuicError, QuicTransport};
pub use wireguard::transport::{WgConnection, WgError, WireGuardTransport};

use crate::crypto::cpace::Role;
use std::net::{Ipv6Addr, SocketAddrV6};
use tokio::io::{AsyncRead, AsyncWrite};

/// 两个传输实现统一的错误类型。
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("wireguard: {0}")]
    WireGuard(#[from] WgError),
    #[error("quic: {0}")]
    Quic(#[from] QuicError),
}

/// 运行时分发：按 [TransportKind] 构造对应实现。
pub enum AnyTransport {
    /// 用户态 WireGuard + smoltcp TCP。
    WireGuard(WireGuardTransport),
    /// QUIC。
    Quic(QuicTransport),
}

impl AnyTransport {
    /// 按 CLI/配置选择构造；发送端默认 QUIC，接收端可自动跟随。
    pub fn new(kind: TransportKind) -> Self {
        match kind {
            TransportKind::WireGuard => AnyTransport::WireGuard(WireGuardTransport::new()),
            TransportKind::Quic => AnyTransport::Quic(QuicTransport::new()),
        }
    }

    /// 当前选择的协议（日志用）。
    pub fn kind(&self) -> TransportKind {
        match self {
            AnyTransport::WireGuard(_) => TransportKind::WireGuard,
            AnyTransport::Quic(_) => TransportKind::Quic,
        }
    }

    /// UDP 绑定地址（`--listen`）。
    pub fn set_udp_bind(&mut self, addr: Option<SocketAddrV6>) {
        match self {
            AnyTransport::WireGuard(t) => t.set_udp_bind(addr),
            AnyTransport::Quic(t) => t.set_udp_bind(addr),
        }
    }

    /// 信令服务器观测到的本端出口 IPv6（server-reflexive 提示）。
    pub fn set_server_hint(&mut self, hint: Option<Ipv6Addr>) {
        match self {
            AnyTransport::WireGuard(t) => t.set_server_hint(hint),
            AnyTransport::Quic(t) => t.set_server_hint(hint),
        }
    }

    /// WireGuard 内层 IP MTU（QUIC 忽略）。
    pub fn set_wg_mtu(&mut self, mtu: usize) {
        if let AnyTransport::WireGuard(t) = self {
            t.set_mtu(mtu);
        }
    }

    /// QUIC 拥塞控制（WireGuard 忽略）。
    pub fn set_quic_congestion(&mut self, congestion: CongestionControl) {
        if let AnyTransport::Quic(t) = self {
            t.set_congestion(congestion);
        }
    }

    /// QUIC 初始 UDP payload（WireGuard 忽略）。
    pub fn set_quic_mtu(&mut self, mtu: Option<u16>) {
        if let AnyTransport::Quic(t) = self
            && let Some(mtu) = mtu
        {
            t.set_initial_mtu(mtu);
        }
    }
}

impl Transport for AnyTransport {
    type Error = TransportError;
    type Connection = AnyConnection;

    fn set_credential_fingerprint(&mut self, fingerprint: [u8; 32]) {
        match self {
            AnyTransport::WireGuard(t) => t.set_credential_fingerprint(fingerprint),
            AnyTransport::Quic(t) => t.set_credential_fingerprint(fingerprint),
        }
    }

    async fn prepare(&mut self) -> Result<Vec<u8>, TransportError> {
        match self {
            AnyTransport::WireGuard(t) => t.prepare().await.map_err(TransportError::WireGuard),
            AnyTransport::Quic(t) => t.prepare().await.map_err(TransportError::Quic),
        }
    }

    async fn establish(
        &mut self,
        role: Role,
        psk: &[u8; 32],
        peer_endpoint: &[u8],
    ) -> Result<AnyConnection, TransportError> {
        match self {
            AnyTransport::WireGuard(t) => t
                .establish(role, psk, peer_endpoint)
                .await
                .map(AnyConnection::WireGuard)
                .map_err(TransportError::WireGuard),
            AnyTransport::Quic(t) => t
                .establish(role, psk, peer_endpoint)
                .await
                .map(AnyConnection::Quic)
                .map_err(TransportError::Quic),
        }
    }
}

/// 统一的连接类型；把两个实现的流装箱成同一个 `AsyncRead + AsyncWrite`。
pub enum AnyConnection {
    /// WireGuard 连接。
    WireGuard(WgConnection),
    /// QUIC 连接。
    Quic(QuicConnection),
}

fn box_stream<S>(stream: S) -> Box<dyn TransportStream>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    Box::new(stream)
}

impl TransportConnection for AnyConnection {
    type Error = TransportError;
    type Stream = Box<dyn TransportStream>;

    fn http_port(&self) -> u16 {
        match self {
            AnyConnection::WireGuard(c) => c.http_port(),
            AnyConnection::Quic(c) => c.http_port(),
        }
    }

    fn peer_http_port(&self) -> u16 {
        match self {
            AnyConnection::WireGuard(c) => c.peer_http_port(),
            AnyConnection::Quic(c) => c.peer_http_port(),
        }
    }

    async fn listen(&mut self) -> Result<Self::Stream, TransportError> {
        match self {
            AnyConnection::WireGuard(c) => c
                .listen()
                .await
                .map(box_stream)
                .map_err(TransportError::WireGuard),
            AnyConnection::Quic(c) => c
                .listen()
                .await
                .map(box_stream)
                .map_err(TransportError::Quic),
        }
    }

    async fn dial(&mut self) -> Result<Self::Stream, TransportError> {
        match self {
            AnyConnection::WireGuard(c) => c
                .dial()
                .await
                .map(box_stream)
                .map_err(TransportError::WireGuard),
            AnyConnection::Quic(c) => c.dial().await.map(box_stream).map_err(TransportError::Quic),
        }
    }

    fn abort_handle(&self) -> AbortHandle {
        match self {
            AnyConnection::WireGuard(c) => c.abort_handle(),
            AnyConnection::Quic(c) => c.abort_handle(),
        }
    }

    async fn shutdown(&mut self) -> Result<(), TransportError> {
        match self {
            AnyConnection::WireGuard(c) => c.shutdown().await.map_err(TransportError::WireGuard),
            AnyConnection::Quic(c) => c.shutdown().await.map_err(TransportError::Quic),
        }
    }

    fn network_stats(&self) -> Option<NetworkStatsHandle> {
        match self {
            AnyConnection::WireGuard(_) => None,
            AnyConnection::Quic(c) => c.network_stats(),
        }
    }
}
