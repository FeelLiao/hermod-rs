//! 信令客户端：WebSocket 会合协议、CPace 握手编排与服务器 SPKI 钉扎。
//! Rust 信令服务实现位于 [`crate::server`]，不与客户端模块混放。

pub mod client;
pub mod device;
pub mod protocol;
pub mod tls;
