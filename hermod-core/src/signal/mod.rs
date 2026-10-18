//! Signaling: the WebSocket rendezvous protocol (client side) and certificate
//! pinning. 服务端仍用 Go 版（D13）；Rust server 延后。See DEVELOPMENT.md §2.1/M2.

pub mod client;
pub mod protocol;
pub mod tls;
