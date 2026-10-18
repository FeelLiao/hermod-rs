//! Hermod core：协议、传输、传输编排与文件传输逻辑。
//!
//! 本 crate 是所有逻辑的唯一归属地，GUI / 脚本 / 其它前端可以直接嵌入，
//! 不必依赖 CLI（DEVELOPMENT.md §3.1，决策 D17）。CLI 只是它的薄封装。
//!
//! # 数据流总览（DEVELOPMENT.md §3.2）
//!
//! ```text
//! 会合（signal）→ 密钥协商（crypto::cpace）→ 加密交换端点清单（transport bundle）
//!   → 建立传输（transport::wireguard）→ 隧道内 HTTP（transport::wireguard::http）
//!   → 传输编排（orchestrate）：tx 发文件 / rx 收文件并裁决
//! ```
//!
//! # 公开模块
//!
//! - [`crypto`]：CPace PAKE（P-256，RFC 9496）、HKDF 派生、AES-256-GCM、
//!   EFF 1296 词表的一次性传输码。**只做密码学原语，不含网络。**
//! - [`signal`]：会合层。WebSocket 线格式（与 Go 版逐字节兼容）、SPKI pinning
//!   的 TLS 客户端、`allocate`/`join`/`wait_ready`/`send_blob`/`recv_blob`
//!   以及两端的 CPace 握手编排。
//! - [`transport`]：传输层。先看 [`transport::iface`] 的 [`Transport`]/
//!   [`TransportConnection`] 契约（上层只依赖它）；具体实现见
//!   [`transport::wireguard`]（用户态 WireGuard + smoltcp 进程内 TCP + 隧道内
//!   HTTP，零特权）。换传输协议时只改实现，不动上层。
//! - [`session`]：会话编排——从**传输码**到已建立的传输连接：
//!   [`session::sender_session`]（tx）/ [`session::receiver_session`]（rx）。
//!   它把「信令 → CPace → bundle 加密交换 → 传输 establish」串成一跳。
//! - [`transfer`]：载荷元数据（`Kind`/文件名/大小/SHA-256）、输入分类、
//!   目标路径安全校验、临时文件路径。
//! - [`config`]：与 Go 版兼容的配置 schema（YAML 键、默认值、Server URL 规范化）。
//! - [`http_meta`]：隧道内 HTTP 的头部约定（`X-Hermod-Kind`、`Content-Disposition`、
//!   帧格式、一次性 URL token）与抗畸形输入校验。
//! - [`orchestrate`]：传输编排——把传输连接与隧道内 HTTP 组合成完整的
//!   tx/rx 主流程：
//!   - [`orchestrate::send_file`] / [`orchestrate::send_text`]：tx 发文件/文本；
//!   - [`orchestrate::receive`]：rx 拉取到临时文件 → 校验 SHA-256 → 原子改名；
//!   - [`orchestrate::serve_file`] / [`orchestrate::fetch_file`]：更底层的
//!     「连接 + HTTP」组合（供自定义流程使用）。
//!
//! # 典型用法（嵌入式）
//!
//! ```no_run
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! use hermod_core::crypto::cpace::Role;
//! use hermod_core::transport::iface::Transport;
//! use hermod_core::transport::wireguard::transport::WireGuardTransport;
//!
//! // 1) 双方各自 prepare，得到要经加密 bundle 交换的端点字节
//! let mut tx = WireGuardTransport::new();
//! let mut rx = WireGuardTransport::new();
//! let tx_ep = tx.prepare().await?;
//! let rx_ep = rx.prepare().await?;
//!
//! // 2) 用 CPace 的 K 派生 PSK（channel_id 来自传输码）
//! let k = [0u8; 32]; // 实际由 CPace finish 得到
//! let psk = hermod_core::crypto::kdf::derive_wg_psk(&k, 42);
//!
//! // 3) establish，得到可 listen/dial 的连接
//! let mut tx_conn = tx.establish(Role::Sender, &psk, &rx_ep).await?;
//! let mut rx_conn = rx.establish(Role::Receiver, &psk, &tx_ep).await?;
//! # Ok(()) }
//! ```
//!
//! 注意：真实的 PSK 必须来自 CPace 握手（[`crypto::cpace`]）后的
//! [`crypto::kdf::derive_wg_psk`]，上面 `k = [0u8; 32]` 只是占位。

pub mod config;
pub mod crypto;
pub mod error;
pub mod http_meta;
pub mod orchestrate;
pub mod session;
pub mod signal;
pub mod transfer;
pub mod transport;

/// 传输层接口契约（便捷再导出）。
///
/// 上层通常只依赖这两个 trait；具体实现见 [`transport::wireguard`]。
pub use transport::iface::{Transport, TransportConnection};
