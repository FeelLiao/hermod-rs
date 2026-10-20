//! Data plane: user-space WireGuard session, IPv6 address discovery, and the
//! smoltcp-based in-process TCP stack. The HTTP/1.1 payload lives in the neutral
//! [crate::transport::http] module. See DEVELOPMENT.md §3.3–§3.5 and M3/M4.

pub mod addr;
pub mod session;
pub mod stack;
pub mod transport;
pub mod udp;
pub mod wg;
