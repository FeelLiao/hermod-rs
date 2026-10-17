//! Signaling: the WebSocket rendezvous protocol shared between server and
//! client, including certificate pinning. See DEVELOPMENT.md §2.1 and M2.

pub mod client;
pub mod protocol;
pub mod ratelimit;
pub mod server;
pub mod store;
pub mod tls;
