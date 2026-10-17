//! 传输层接口（契约）。
//!
//! 上层只依赖本模块定义的 [`Transport`] trait；换传输协议 = 换实现，不动上层。
//!
//! ## 契约概要（下一步落地为 trait 定义）
//!
//! 一次载荷传输的生命周期：
//!
//! 1. 会合层经 CPace 得到 K（[`crate::crypto::cpace`]）；
//! 2. 用 K 派生预共享密钥 PSK（[`crate::crypto::kdf::derive_wg_psk`]）；
//! 3. 双方交换用 K 加密的端点 bundle（[`crate::transport::wireguard::session`]）；
//! 4. **传输层**：用 bundle 里的对端端点 + PSK 建立连接，返回一条双向字节流；
//! 5. 上层在这条流上跑载荷协议（tx 服务 / rx 拨号），完成后关闭。
//!
//! ## 接口暂定形状（实现时定稿）
//!
//! ```text
//! trait Transport {
//!     type Error: std::error::Error;
//!     type Connection;                       // 字节流 + tunnel 内地址
//!
//!     /// 建立到对端的传输会话。
//!     /// - bundle: 对端公布的端点（M2.4 EndpointBundle）
//!     /// - psk:    HKDF(K, salt=channel_id, info="hermod-wg-psk-v1")（kdf.rs）
//!     /// - role:   Sender（tx，监听）/ Receiver（rx，拨号）
//!     fn establish(&self, bundle: &EndpointBundle, psk: &[u8; 32], role: Role)
//!         -> impl Future<Output = Result<Self::Connection, Self::Error>>;
//! }
//! ```
//!
//! 接口设计时要注意的约束（来自 DEVELOPMENT.md）：
//! - **零特权**（D14）：传输层不得要求 root，不得装宿主机路由/接口；
//! - **端到端加密**：载荷经过传输层时已由传输层自身加密（WireGuard），
//!   上层不再加密（D8）；
//! - **角色**：tx 负责在隧道内监听 HTTP，rx 拨号（§3.4）。
//!
//! 待 M3 开始实现时，本文件会包含真正的 `Transport` trait 定义。
