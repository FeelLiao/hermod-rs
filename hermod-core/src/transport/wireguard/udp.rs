//! WireGuard 外层 UDP：自动双栈绑定，并支持 IPv4-only 系统。

use std::net::{Ipv6Addr, SocketAddr};
use tokio::net::UdpSocket;

/// 自动绑定双栈；IPv6 不可用时仍可使用 IPv4。
pub async fn bind(port: u16) -> std::io::Result<UdpSocket> {
    let address = (port != 0).then(|| SocketAddr::new(Ipv6Addr::UNSPECIFIED.into(), port));
    UdpSocket::from_std(crate::transport::network::bind_udp(address)?)
}

/// 指定地址绑定供内部测试使用。
pub async fn bind_at(addr: impl Into<SocketAddr>) -> std::io::Result<UdpSocket> {
    UdpSocket::from_std(crate::transport::network::bind_udp(Some(addr.into()))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddrV6;

    /// `bind_at` 会绑定到指定地址（内部测试用）。
    #[tokio::test]
    async fn bind_at_honors_address() {
        let addr = SocketAddrV6::new(Ipv6Addr::LOCALHOST, 0, 0, 0);
        let sock = bind_at(addr).await.unwrap();
        let local = sock.local_addr().unwrap();
        assert!(local.is_ipv6());
        assert_eq!(local.ip(), std::net::IpAddr::V6(Ipv6Addr::LOCALHOST));
    }

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
