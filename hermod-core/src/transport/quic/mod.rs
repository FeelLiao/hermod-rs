//! QUIC 传输实现（默认数据面）。
//!
//! 与 [crate::transport::wireguard] **完全独立**：自己的 endpoint bundle、自己的
//! IPv6 候选逻辑、自己的证书/握手代码，不引用 WireGuard 的任何类型。
//!
//! - [transport::QuicTransport]：实现 [crate::transport::iface::Transport]；
//! - [transport::QuicConnection]：实现 [crate::transport::iface::TransportConnection]；
//! - [auth]：每会话临时自签名证书 + server 证书指纹钉扎；
//! - 流级认证：每条双向流开头传 32 字节 session 指纹（SHA-256(token)）；
//! - [bundle]：QUIC 自己的 [bundle::QuicEndpoint]；
//! - [addr]：IPv6 候选枚举/排序。

pub mod addr;
pub mod auth;
pub mod bundle;
pub mod transport;

pub use transport::{CongestionControl, QuicConnection, QuicError, QuicTransport};
