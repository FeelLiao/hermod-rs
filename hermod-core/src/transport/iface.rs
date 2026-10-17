//! 传输层接口（契约）。
//!
//! 上层只依赖本模块的 [`Transport`] / [`TransportConnection`]；
//! 换传输协议 = 换实现，不动上层。
//!
//! ## 端点信息为什么是 opaque 字节
//!
//! 不同传输协议要公布的端点完全不同（WireGuard 公布公钥+UDP端口+IPv6候选，
//! QUIC 可能公布证书/地址）。因此接口只把端点当作**传输层自定义的 opaque 字节**：
//! 上层负责搬运（塞进加密 bundle）与加密，但**不解析**。这样换协议时上层零改动。

use crate::crypto::cpace::Role;
use core::future::Future;
use tokio::io::{AsyncRead, AsyncWrite};

/// 传输层：负责把载荷字节在两端之间可靠地搬运。
///
/// 生命周期（对应 DEVELOPMENT.md §3.2 步骤 5–7）：
/// 1. [`prepare`](Transport::prepare)：准备本端端点，返回要公布给对端的字节；
/// 2. 上层把该字节用 CPace 密钥 AEAD 加密后经会合层交换；
/// 3. [`establish`](Transport::establish)：用对端端点 + PSK + 角色建立连接；
/// 4. 在返回的 [`TransportConnection`] 上跑载荷协议（tx `listen` / rx `dial`）。
pub trait Transport {
    /// 初始化/建连错误。
    type Error: std::error::Error + Send + Sync + 'static;

    /// 已建立的传输连接类型。
    type Connection: TransportConnection;

    /// 初始化传输层：生成会话语料（密钥、端口、本机地址等），
    /// 返回**本端要公布给对端的端点字节**（传输层自定义格式，上层不解析）。
    fn prepare(&mut self) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send;

    /// 用对端端点 + PSK + 角色建立传输连接。
    ///
    /// - `role`：`Sender`（tx，建立后负责 [`TransportConnection::listen`]）
    ///   或 `Receiver`（rx，负责 [`TransportConnection::dial`]）；
    /// - `psk`：由 CPace 的 K 派生（见 [`crate::crypto::kdf::derive_wg_psk`]），
    ///   两端各自本地派生，**从不过网**；
    /// - `peer_endpoint`：对端 [`Transport::prepare`] 的返回值（已 AEAD 解密）。
    fn establish(
        &mut self,
        role: Role,
        psk: &[u8; 32],
        peer_endpoint: &[u8],
    ) -> impl Future<Output = Result<Self::Connection, Self::Error>> + Send;
}

/// 已建立的传输连接：在隧道内做 TCP 式收发。
///
/// 载荷协议（隧道内 HTTP/1.1）直接跑在 [`listen`](TransportConnection::listen) /
/// [`dial`](TransportConnection::dial) 返回的字节流上，不感知底层是 WireGuard。
pub trait TransportConnection {
    /// 连接层错误。
    type Error: std::error::Error + Send + Sync + 'static;

    /// 隧道内字节流（可直接喂给 HTTP 的 AsyncRead/AsyncWrite）。
    type Stream: AsyncRead + AsyncWrite + Unpin + Send;

    /// 本端公布的 HTTP 端口（tx 用它监听）。
    fn http_port(&self) -> u16;

    /// 对端公布的 HTTP 端口（rx 用它拨号）。
    fn peer_http_port(&self) -> u16;

    /// tx：在隧道内监听，等待对端连接并返回字节流。
    fn listen(&mut self) -> impl Future<Output = Result<Self::Stream, Self::Error>> + Send;

    /// rx：向隧道内的对端拨号并返回字节流。
    fn dial(&mut self) -> impl Future<Output = Result<Self::Stream, Self::Error>> + Send;
}

#[cfg(test)]
mod tests {
    //! 接口本身没有逻辑可测；这里用一个最小 mock 证明 trait 形状可用
    //! （也验证 `impl Future + Send` 在 tokio 里能正常 await）。

    use super::*;

    struct MockTransport;

    struct MockConnection;

    struct MockStream;

    impl AsyncRead for MockStream {
        fn poll_read(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            _buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Pending
        }
    }

    impl AsyncWrite for MockStream {
        fn poll_write(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            _buf: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            std::task::Poll::Pending
        }
        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    impl TransportConnection for MockConnection {
        type Error = std::io::Error;
        type Stream = MockStream;

        fn http_port(&self) -> u16 {
            1
        }
        fn peer_http_port(&self) -> u16 {
            2
        }
        async fn listen(&mut self) -> Result<Self::Stream, Self::Error> {
            Ok(MockStream)
        }
        async fn dial(&mut self) -> Result<Self::Stream, Self::Error> {
            Ok(MockStream)
        }
    }

    impl Transport for MockTransport {
        type Error = std::io::Error;
        type Connection = MockConnection;

        async fn prepare(&mut self) -> Result<Vec<u8>, Self::Error> {
            Ok(vec![1, 2, 3])
        }
        async fn establish(
            &mut self,
            _role: Role,
            _psk: &[u8; 32],
            _peer_endpoint: &[u8],
        ) -> Result<Self::Connection, Self::Error> {
            Ok(MockConnection)
        }
    }

    /// 用泛型函数消费 trait，证明接口可以泛型注入（换实现 = 换类型参数）。
    async fn run<C: Transport>(mut t: C) -> Result<(u16, u16), C::Error> {
        let local = t.prepare().await?;
        let mut conn = t.establish(Role::Sender, &[0u8; 32], &local).await?;
        // 注意：Connection 的错误类型与 Transport 独立（两个 trait 各有 Error）。
        let _ = conn.listen().await.expect("mock listen");
        Ok((conn.http_port(), conn.peer_http_port()))
    }

    #[tokio::test]
    async fn trait_shape_is_usable() {
        let (lp, pp) = run(MockTransport).await.unwrap();
        assert_eq!((lp, pp), (1, 2));
    }
}
