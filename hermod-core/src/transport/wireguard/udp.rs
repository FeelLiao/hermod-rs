//! 外层 UDP socket（M3.3）：绑定 `[::]` 并置 `IPV6_V6ONLY`。
//!
//! 对应 DEVELOPMENT.md §3.3「地址族」：只走 IPv6，socket 绑 `[::]` 并置
//! `IPV6_V6ONLY`，行为确定，不会意外收到 v4-mapped 流量。
//!
//! 说明：`std`/`tokio` 不暴露 `IPV6_V6ONLY`，这里用已在依赖树里的 `socket2`
//! （安全封装，无需 unsafe）。批处理收发（recvmmsg/GSO）属 M5.1 优化，暂不实现。

use socket2::{Domain, Socket, Type};
use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6};
use tokio::net::UdpSocket;

/// 绑定 `[::]:port`（port=0 时由内核分配）的 IPv6-only UDP socket。
pub async fn bind(port: u16) -> std::io::Result<UdpSocket> {
    let sock = Socket::new(Domain::IPV6, Type::DGRAM, None)?;
    // 只收 IPv6，避免 v4-mapped 流量（§3.3）。
    sock.set_only_v6(true)?;
    sock.set_nonblocking(true)?;
    let addr = SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, port, 0, 0));
    sock.bind(&addr.into())?;
    let std_udp: std::net::UdpSocket = sock.into();
    UdpSocket::from_std(std_udp)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 绑定到随机端口，确认是 IPv6 socket 且能做本机回环收发。
    #[tokio::test]
    async fn binds_ipv6_and_roundtrips() {
        let a = bind(0).await.unwrap();
        let a_port = a.local_addr().unwrap().port();
        assert!(a.local_addr().unwrap().is_ipv6());

        let b = bind(0).await.unwrap();
        let dst = SocketAddrV6::new(Ipv6Addr::LOCALHOST, a_port, 0, 0);
        b.send_to(b"ping", dst).await.unwrap();

        let mut buf = [0u8; 16];
        let (n, _src) = a.recv_from(&mut buf).await.unwrap();
        assert_eq!(&buf[..n], b"ping");
    }
}
