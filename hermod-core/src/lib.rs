//! Hermod core library: all protocol, tunnel, and transfer logic lives here so
//! that alternative clients (GUI, scripts, etc.) can embed Hermod without
//! depending on the CLI crate (DEVELOPMENT.md §3.1, decision D17).
//!
//! Public API surface (a stable entry point for third-party clients):
//! - [`crypto`]   — CPace PAKE, key derivation, AEAD, transfer-code wordlist
//! - [`signal`]   — signaling server & client (wire protocol, TLS pinning)
//! - [`transport`]— 传输层：抽象接口 + 具体实现（当前为用户态 WireGuard + TCP/HTTP）
//! - [`transfer`] — path safety, integrity flow, and tx/rx orchestration API
//! - [`config`]   — Go-compatible configuration schema
//! - [`http_meta`]— in-tunnel HTTP header conventions
//! - [`error`]    — unified error type

pub mod config;
pub mod crypto;
pub mod error;
pub mod http_meta;
pub mod signal;
pub mod transfer;
pub mod transport;
