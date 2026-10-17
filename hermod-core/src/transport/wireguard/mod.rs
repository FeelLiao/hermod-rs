//! Data plane: user-space WireGuard session, IPv6 address discovery, the
//! smoltcp-based in-process TCP stack, and in-tunnel HTTP.
//! See DEVELOPMENT.md §3.3–§3.5 and M3/M4.

pub mod addr;
pub mod http;
pub mod session;
pub mod stack;
pub mod udp;
pub mod wg;
