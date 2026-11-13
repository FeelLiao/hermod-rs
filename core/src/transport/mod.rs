//! 传输层（data plane）：抽象「一条已认证的双向字节流」。
//!
//! 目前唯一的实现是 [quic]（QUIC + TLS 1.3 + 可插拔拥塞控制，默认 BBR）。
//! [Transport] / [TransportConnection] trait 与 [TransportKind] 线上码是预留的
//! 扩展点：未来若新增传输协议，只需在 [TransportKind] 里加 wire 码并提供新的
//! [Transport] 实现，`session` / `orchestrate` 不用改动。
//!
//! ## 目录
//!
//! ```text
//! transport/
//! ├── iface.rs            # trait + TransportKind + AbortHandle
//! ├── mod.rs              # AnyTransport / AnyConnection 运行时分发
//! ├── network.rs          # 传输无关的地址/UDP 工具
//! └── quic/               # 实现：QUIC endpoint / TLS 钉扎 / 可插拔 CC
//! ```

pub mod iface;
pub mod network;
pub mod quic;

pub use iface::{
    AbortHandle, NetworkStats, NetworkStatsHandle, Transport, TransportConnection, TransportKind,
    TransportStream,
};
pub use quic::transport::{CongestionControl, QuicConnection, QuicError, QuicTransport};

use crate::crypto::cpace::Role;
use std::net::{Ipv6Addr, SocketAddr};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};

/// 两个传输实现统一的错误类型。
///
/// 结构保留多臂形态以备将来新增传输实现。
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("quic: {0}")]
    Quic(#[from] QuicError),
}

/// 运行时分发：按 [TransportKind] 构造对应实现。
pub enum AnyTransport {
    /// QUIC（当前唯一实现）。
    Quic(QuicTransport),
}

impl AnyTransport {
    /// 按 CLI/配置选择构造；当前恒为 QUIC。
    pub fn new(kind: TransportKind) -> Self {
        match kind {
            TransportKind::Quic => Self::Quic(QuicTransport::new()),
        }
    }

    /// 当前选择的协议（日志用）。
    pub fn kind(&self) -> TransportKind {
        TransportKind::Quic
    }

    /// UDP 绑定地址（内部测试用）。
    pub fn set_udp_bind(&mut self, addr: Option<SocketAddr>) {
        match self {
            Self::Quic(t) => t.set_udp_bind(addr),
        }
    }

    /// 信令服务器观测到的本端出口 IPv6（server-reflexive 提示）。
    pub fn set_server_hint(&mut self, hint: Option<Ipv6Addr>) {
        match self {
            Self::Quic(t) => t.set_server_hint(hint),
        }
    }

    /// 配置本次会话的地址族与本端传输预算。
    pub fn set_transfer_options(
        &mut self,
        family: crate::options::IpFamily,
        tuning: crate::options::TransferTuning,
    ) {
        match self {
            Self::Quic(t) => t.set_transfer_options(family, tuning),
        }
    }

    /// 统一选项已由会话层校验，只配置当前协议。
    pub fn configure(
        &mut self,
        mtu: Option<usize>,
        congestion: crate::options::Congestion,
        profile: crate::options::LinkProfile,
    ) {
        use crate::options::Congestion as C;
        match self {
            Self::Quic(t) => {
                t.set_mtu_limit(mtu.unwrap_or(1452) as u16);
                t.set_congestion(match congestion {
                    C::Cubic => CongestionControl::Cubic,
                    C::Newreno => CongestionControl::NewReno,
                    _ => CongestionControl::Bbr,
                });
                t.set_link_profile(profile);
            }
        }
    }
}

impl Transport for AnyTransport {
    type Error = TransportError;
    type Connection = AnyConnection;

    fn set_credential_fingerprint(&mut self, fingerprint: [u8; 32]) {
        match self {
            Self::Quic(t) => t.set_credential_fingerprint(fingerprint),
        }
    }

    async fn prepare(&mut self) -> Result<Vec<u8>, TransportError> {
        match self {
            Self::Quic(t) => t.prepare().await.map_err(TransportError::Quic),
        }
    }

    async fn establish(
        &mut self,
        role: Role,
        psk: &[u8; 32],
        peer_endpoint: &[u8],
    ) -> Result<AnyConnection, TransportError> {
        match self {
            Self::Quic(t) => t
                .establish(role, psk, peer_endpoint)
                .await
                .map(AnyConnection::Quic)
                .map_err(TransportError::Quic),
        }
    }
}

/// 统一的连接类型；把实现的流装箱成同一个 `AsyncRead + AsyncWrite`。
pub enum AnyConnection {
    /// QUIC 连接（当前唯一实现）。
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
            Self::Quic(c) => c.http_port(),
        }
    }

    fn peer_http_port(&self) -> u16 {
        match self {
            Self::Quic(c) => c.peer_http_port(),
        }
    }

    async fn listen(&mut self) -> Result<Self::Stream, TransportError> {
        match self {
            Self::Quic(c) => c
                .listen()
                .await
                .map(box_stream)
                .map_err(TransportError::Quic),
        }
    }

    async fn dial(&mut self) -> Result<Self::Stream, TransportError> {
        match self {
            Self::Quic(c) => c.dial().await.map(box_stream).map_err(TransportError::Quic),
        }
    }

    fn abort_handle(&self) -> AbortHandle {
        match self {
            Self::Quic(c) => c.abort_handle(),
        }
    }

    async fn shutdown(&mut self) -> Result<(), TransportError> {
        match self {
            Self::Quic(c) => c.shutdown().await.map_err(TransportError::Quic),
        }
    }

    fn network_stats(&self) -> Option<NetworkStatsHandle> {
        match self {
            Self::Quic(c) => c.network_stats(),
        }
    }

    fn peer_addr(&self) -> Option<SocketAddr> {
        match self {
            Self::Quic(c) => c.peer_addr(),
        }
    }

    fn transfer_tuning(&self) -> crate::options::TransferTuning {
        match self {
            Self::Quic(c) => c.transfer_tuning(),
        }
    }

    async fn rtt(&mut self) -> Option<Duration> {
        match self {
            Self::Quic(c) => c.rtt().await,
        }
    }
}
