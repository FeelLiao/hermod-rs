//! 传输层（data plane）：抽象「一条已认证的双向字节流」，与具体传输协议解耦。
//!
//! ## 为什么要抽象
//!
//! 会合层（[`crate::signal`]）拿到共享密钥 K 之后，双方需要一条能跑载荷协议
//! （隧道内 HTTP）的连接。这条连接**当前**由用户态 WireGuard + 进程内 TCP 提供，
//! 但上层（载荷协议 + [`crate::transfer`] 语义）不应该绑定 WireGuard。
//!
//! 因此传输层对外只暴露一个接口（[`iface::Transport`]）：
//! - 将来要换传输协议（例如换成 QUIC、或加 TCP 中继），
//!   **实现同一个接口即可直接替换**，上层代码不感知；
//! - 具体实现放在子模块里，当前是 [`wireguard`]。
//!
//! ## 目录
//!
//! ```text
//! transport/
//! ├── iface.rs      # 接口（Transport trait）—— 上层只依赖它
//! └── wireguard/    # 当前实现：用户态 WireGuard + smoltcp TCP + 隧道内 HTTP
//!     ├── wg/       #   X25519 密钥、boringtun 隧道封装
//!     ├── stack/    #   smoltcp 设备桥接
//!     ├── http/     #   隧道内 HTTP/1.1
//!     ├── udp.rs    #   外层 UDP socket
//!     ├── addr.rs   #   本机 IPv6 枚举
//!     └── session.rs#   会话编排（bundle → WG → HTTP）
//! ```

pub mod iface;
pub mod wireguard;
