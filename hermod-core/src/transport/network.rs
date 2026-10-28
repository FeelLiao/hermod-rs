//! 两种传输共用地址族工具；IPv4 仅发布局域网地址，不推测公网 NAT 映射。
use socket2::{Domain, Socket, Type};
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6};

pub fn local_ipv4() -> Vec<Ipv4Addr> {
    let mut addresses = Vec::new();
    for interface in if_addrs::get_if_addrs().unwrap_or_default() {
        if !interface.is_oper_up() || interface.is_loopback() {
            continue;
        }
        if let if_addrs::IfAddr::V4(address) = interface.addr
            && (address.ip.is_private() || address.ip.is_link_local())
        {
            // 未显式设置广播地址的接口可能返回自身地址，不能据此排除有效单播地址。
            addresses.push(address.ip);
        }
    }
    addresses.sort_unstable();
    addresses.dedup();
    addresses.truncate(32);
    addresses
}

/// 自动使用显式双栈 socket；没有 IPv6 栈的系统仍能绑定 IPv4。
pub fn bind_udp(bind: Option<SocketAddr>) -> io::Result<std::net::UdpSocket> {
    fn bind_at(address: SocketAddr) -> io::Result<std::net::UdpSocket> {
        let socket = Socket::new(
            if address.is_ipv6() {
                Domain::IPV6
            } else {
                Domain::IPV4
            },
            Type::DGRAM,
            None,
        )?;
        if address.is_ipv6() {
            socket.set_only_v6(!address.ip().is_unspecified())?;
        }
        socket.set_nonblocking(true)?;
        let _ = socket.set_recv_buffer_size(8 * 1024 * 1024);
        let _ = socket.set_send_buffer_size(8 * 1024 * 1024);
        socket.bind(&address.into())?;
        Ok(socket.into())
    }
    match bind {
        Some(address) => bind_at(address),
        None => bind_at(SocketAddr::new(Ipv6Addr::UNSPECIFIED.into(), 0))
            .or_else(|_| bind_at(SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), 0))),
    }
}

/// 指定地址只发布自身；自动双栈绑定同时发布两种地址。
pub fn candidates(bound: SocketAddr, mut ipv6: Vec<Ipv6Addr>) -> (Vec<Ipv6Addr>, Vec<Ipv4Addr>) {
    if !bound.ip().is_unspecified() {
        return match bound.ip() {
            IpAddr::V4(ip) => (Vec::new(), vec![ip]),
            IpAddr::V6(ip) => (vec![ip], Vec::new()),
        };
    }
    if bound.is_ipv4() {
        ipv6.clear();
    }
    ipv6.dedup();
    ipv6.truncate(32);
    (ipv6, local_ipv4())
}

/// 旧端的 IPv6 字段仍然有效；新端先尝试 IPv4 局域网路径。
pub fn peer_addresses(
    ipv6: &[Ipv6Addr],
    ipv4: &[Ipv4Addr],
    port: u16,
    bound: SocketAddr,
) -> io::Result<Vec<SocketAddr>> {
    if port == 0 || ipv6.len() + ipv4.len() > 64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid peer address list",
        ));
    }
    let supports_v4 = bound.is_ipv4() || bound.ip().is_unspecified();
    let mut addresses = Vec::new();
    if supports_v4 {
        addresses.extend(
            ipv4.iter()
                .filter(|ip| {
                    ip.is_private()
                        || ip.is_link_local()
                        || (ip.is_loopback() && bound.ip().is_loopback())
                })
                .map(|ip| SocketAddr::new((*ip).into(), port)),
        );
    }
    if bound.is_ipv6() {
        addresses.extend(
            ipv6.iter()
                .filter(|ip| {
                    !ip.is_unspecified()
                        && !ip.is_multicast()
                        && ip.to_ipv4_mapped().is_none()
                        && !ip.is_unicast_link_local()
                })
                .map(|ip| SocketAddr::new((*ip).into(), port)),
        );
    }
    if addresses.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            "No compatible peer IPv4/IPv6 candidates",
        ));
    }
    Ok(addresses)
}

/// 统一 WireGuard 端点表示，避免同一 IPv4 地址以两种形式参与漫游。
pub fn canonical(address: SocketAddr) -> SocketAddr {
    match address {
        SocketAddr::V6(v6) if v6.ip().to_ipv4_mapped().is_some() => {
            SocketAddr::new(v6.ip().to_ipv4_mapped().unwrap().into(), v6.port())
        }
        _ => address,
    }
}

/// 双栈 socket 显式使用映射地址发包，不依赖操作系统接受跨族 sockaddr。
pub fn for_socket(address: SocketAddr, ipv6: bool) -> SocketAddr {
    match address {
        SocketAddr::V4(v4) if ipv6 => {
            SocketAddrV6::new(v4.ip().to_ipv6_mapped(), v4.port(), 0, 0).into()
        }
        _ => address,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn automatic_socket_accepts_ipv4_and_ipv6() {
        let socket = tokio::net::UdpSocket::from_std(bind_udp(None).unwrap()).unwrap();
        for source in ["127.0.0.1:0", "[::1]:0"] {
            if source.starts_with('[') && socket.local_addr().unwrap().is_ipv4() {
                continue;
            }
            let peer = tokio::net::UdpSocket::bind(source).await.unwrap();
            let target = SocketAddr::new(
                peer.local_addr().unwrap().ip(),
                socket.local_addr().unwrap().port(),
            );
            peer.send_to(b"ping", target).await.unwrap();
            let mut buffer = [0; 16];
            let (length, from) = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                socket.recv_from(&mut buffer),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(&buffer[..length], b"ping");
            let from = canonical(from);
            assert_eq!(from, peer.local_addr().unwrap());
            socket
                .send_to(
                    b"pong",
                    for_socket(from, socket.local_addr().unwrap().is_ipv6()),
                )
                .await
                .unwrap();
            let length =
                tokio::time::timeout(std::time::Duration::from_secs(2), peer.recv(&mut buffer))
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(&buffer[..length], b"pong");
        }
    }

    #[test]
    fn peer_candidates_are_bounded_and_match_the_local_socket() {
        let local = "0.0.0.0:1234".parse().unwrap();
        let ipv6 = ["fd00::1".parse().unwrap()];
        let ipv4 = ["192.168.1.2".parse().unwrap()];
        assert_eq!(
            peer_addresses(&ipv6, &ipv4, 42, local).unwrap(),
            vec!["192.168.1.2:42".parse::<SocketAddr>().unwrap()]
        );
        assert!(peer_addresses(&ipv6, &[], 42, local).is_err());
        assert!(peer_addresses(&[], &["8.8.8.8".parse().unwrap()], 42, local).is_err());
        assert!(peer_addresses(&[], &ipv4, 0, local).is_err());
        assert!(peer_addresses(&[], &vec![ipv4[0]; 65], 42, local).is_err());
    }
}
