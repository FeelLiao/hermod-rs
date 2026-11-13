//! 两种传输共用地址族工具；IPv4 仅发布局域网地址，不推测公网 NAT 映射。
use crate::options::IpFamily;
use socket2::{Domain, Socket, Type};
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6};

/// UDP 收发缓冲请求值（≥ 局域网 BDP；内核可能钳制，失败只告警不失败）。
const UDP_BUFFER: usize = 16 * 1024 * 1024;

/// 设置套接字缓冲；设置失败记告警（不阻断传输）。
fn set_buffer(setter: impl FnOnce(usize) -> io::Result<()>, bytes: usize, what: &str) {
    if let Err(error) = setter(bytes) {
        crate::log_warn!(target = what, error = %error, "UDP buffer size request failed");
    }
}

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
    bind_udp_with_options(bind, IpFamily::Auto, UDP_BUFFER)
}

/// 按地址族绑定；缓冲大小单位为字节，显式地址与地址族冲突时直接拒绝。
pub fn bind_udp_with_options(
    bind: Option<SocketAddr>,
    family: IpFamily,
    buffer_bytes: usize,
) -> io::Result<std::net::UdpSocket> {
    bind_socket(bind, 0, family, Type::DGRAM, Some(buffer_bytes)).map(Into::into)
}

/// TCP 服务端绑定；Auto 显式双栈，IPv6 不可用时才退回 IPv4。
pub fn bind_tcp(port: u16, family: IpFamily) -> io::Result<tokio::net::TcpListener> {
    let socket = bind_socket(None, port, family, Type::STREAM, None)?;
    socket.listen(128)?;
    tokio::net::TcpListener::from_std(socket.into())
}

fn bind_socket(
    bind: Option<SocketAddr>,
    port: u16,
    family: IpFamily,
    kind: Type,
    buffer: Option<usize>,
) -> io::Result<Socket> {
    let bind_at = |address: SocketAddr| -> io::Result<Socket> {
        let address = canonical(address);
        if !family.allows(address.ip()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Bind address {address} conflicts with IP family {family:?}"),
            ));
        }
        let socket = Socket::new(
            if address.is_ipv6() {
                Domain::IPV6
            } else {
                Domain::IPV4
            },
            kind,
            None,
        )?;
        if address.is_ipv6() {
            socket.set_only_v6(family == IpFamily::V6 || !address.ip().is_unspecified())?;
        }
        socket.set_nonblocking(true)?;
        if let Some(bytes) = buffer {
            set_buffer(|n| socket.set_recv_buffer_size(n), bytes, "rcvbuf");
            set_buffer(|n| socket.set_send_buffer_size(n), bytes, "sndbuf");
        }
        socket.bind(&address.into())?;
        Ok(socket)
    };
    if let Some(address) = bind {
        return bind_at(address);
    }
    match family {
        IpFamily::V4 => bind_at(SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), port)),
        IpFamily::V6 => bind_at(SocketAddr::new(Ipv6Addr::UNSPECIFIED.into(), port)),
        IpFamily::Auto => bind_at(SocketAddr::new(Ipv6Addr::UNSPECIFIED.into(), port))
            .or_else(|_| bind_at(SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), port))),
    }
}

/// 指定地址只发布自身；自动双栈绑定同时发布两种地址。
pub fn candidates(bound: SocketAddr, ipv6: Vec<Ipv6Addr>) -> (Vec<Ipv6Addr>, Vec<Ipv4Addr>) {
    candidates_with_family(bound, ipv6, IpFamily::Auto)
}

/// 仅发布所选地址族、实际绑定 socket 可达的候选。
pub fn candidates_with_family(
    bound: SocketAddr,
    mut ipv6: Vec<Ipv6Addr>,
    family: IpFamily,
) -> (Vec<Ipv6Addr>, Vec<Ipv4Addr>) {
    let bound = canonical(bound);
    if !family.allows(bound.ip()) && !bound.ip().is_unspecified() {
        return (Vec::new(), Vec::new());
    }
    if !bound.ip().is_unspecified() {
        return match bound.ip() {
            IpAddr::V4(ip) => (Vec::new(), vec![ip]),
            IpAddr::V6(ip) => (vec![ip], Vec::new()),
        };
    }
    if bound.is_ipv4() || family == IpFamily::V4 {
        ipv6.clear();
    }
    ipv6.retain(|ip| {
        !ip.is_unspecified()
            && !ip.is_multicast()
            && !ip.is_unicast_link_local()
            && ip.to_ipv4_mapped().is_none()
    });
    ipv6.sort_unstable();
    ipv6.dedup();
    ipv6.truncate(32);
    let ipv4 = if family == IpFamily::V6 {
        Vec::new()
    } else {
        local_ipv4()
    };
    (ipv6, ipv4)
}

/// 旧端的 IPv6 字段仍然有效；新端先尝试 IPv4 局域网路径。
pub fn peer_addresses(
    ipv6: &[Ipv6Addr],
    ipv4: &[Ipv4Addr],
    port: u16,
    bound: SocketAddr,
) -> io::Result<Vec<SocketAddr>> {
    peer_addresses_with_family(ipv6, ipv4, port, bound, IpFamily::Auto)
}

/// 过滤远端候选；强制地址族绝不跨族回退。
pub fn peer_addresses_with_family(
    ipv6: &[Ipv6Addr],
    ipv4: &[Ipv4Addr],
    port: u16,
    bound: SocketAddr,
    family: IpFamily,
) -> io::Result<Vec<SocketAddr>> {
    if port == 0 || ipv6.len() + ipv4.len() > 64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid peer address list",
        ));
    }
    let supports_v4 = family != IpFamily::V6 && (bound.is_ipv4() || bound.ip().is_unspecified());
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
    if bound.is_ipv6() && family != IpFamily::V4 {
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
            format!(
                "No compatible peer candidates for IP family {family:?} and local bind {bound}"
            ),
        ));
    }
    Ok(addresses)
}

/// 统一端点表示，避免同一 IPv4 地址以两种形式参与漫游（IPv4/IPv6 隧道双族）。
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

/// 地址性质，而非 RTT 推测；映射 IPv4 与普通 IPv4 一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    Lan,
    Wan,
}

pub fn classify_link(address: SocketAddr) -> LinkKind {
    let local = match canonical(address).ip() {
        IpAddr::V4(ip) => ip.is_private() || ip.is_link_local() || ip.is_loopback(),
        IpAddr::V6(ip) => ip.is_unique_local() || ip.is_unicast_link_local() || ip.is_loopback(),
    };
    if local { LinkKind::Lan } else { LinkKind::Wan }
}

/// 显式 DNS 解析后过滤，避免底层连接器自行跨族回退。
pub(crate) async fn connect_tcp_with_family(
    host: &str,
    port: u16,
    family: IpFamily,
) -> io::Result<tokio::net::TcpStream> {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let addresses: Vec<_> = tokio::net::lookup_host((host, port))
        .await?
        .map(canonical)
        .filter(|address| family.allows(address.ip()))
        .collect();
    if addresses.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            format!("No DNS candidates for {host}:{port} matching IP family {family:?}"),
        ));
    }
    let mut last_error = None;
    for address in addresses {
        match tokio::net::TcpStream::connect(address).await {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.expect("nonempty DNS candidates"))
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
    fn strict_family_filters_candidates_and_conflicting_binds() {
        let ipv6 = ["fd00::1".parse().unwrap()];
        let ipv4 = ["192.168.1.2".parse().unwrap()];
        let bound = "[::]:42".parse().unwrap();
        let v6 = peer_addresses_with_family(&ipv6, &ipv4, 42, bound, IpFamily::V6).unwrap();
        assert!(v6.iter().all(SocketAddr::is_ipv6));
        let v4 = peer_addresses_with_family(&ipv6, &ipv4, 42, bound, IpFamily::V4).unwrap();
        assert!(v4.iter().all(SocketAddr::is_ipv4));
        assert!(peer_addresses_with_family(&[], &ipv4, 42, bound, IpFamily::V6).is_err());
        assert!(peer_addresses_with_family(&ipv6, &[], 42, bound, IpFamily::V4).is_err());
        assert!(
            candidates_with_family(bound, ipv6.to_vec(), IpFamily::V6)
                .1
                .is_empty()
        );
        assert!(
            candidates_with_family(bound, ipv6.to_vec(), IpFamily::V4)
                .0
                .is_empty()
        );
        for (addr, family) in [("[::]:0", IpFamily::V4), ("0.0.0.0:0", IpFamily::V6)] {
            assert_eq!(
                bind_udp_with_options(Some(addr.parse().unwrap()), family, 65536)
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::InvalidInput
            );
        }
    }

    #[test]
    fn link_classification_uses_address_scope() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.1.1",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:192.168.1.1",
        ] {
            assert_eq!(
                classify_link(SocketAddr::new(ip.parse().unwrap(), 42)),
                LinkKind::Lan,
                "{ip}"
            );
        }
        for ip in ["8.8.8.8", "100.64.0.1", "2001:4860::1", "::ffff:8.8.8.8"] {
            assert_eq!(
                classify_link(SocketAddr::new(ip.parse().unwrap(), 42)),
                LinkKind::Wan,
                "{ip}"
            );
        }
    }

    #[tokio::test]
    async fn strict_udp_socket_family_flags_and_roundtrip() {
        for family in [IpFamily::V4, IpFamily::V6] {
            let socket = bind_udp_with_options(None, family, 65536).unwrap();
            assert_eq!(
                socket.local_addr().unwrap().is_ipv6(),
                family == IpFamily::V6
            );
            if family == IpFamily::V6 {
                assert!(socket2::SockRef::from(&socket).only_v6().unwrap());
            }
            let socket = tokio::net::UdpSocket::from_std(socket).unwrap();
            let ip = if family == IpFamily::V6 {
                "::1"
            } else {
                "127.0.0.1"
            };
            let peer = tokio::net::UdpSocket::bind(SocketAddr::new(ip.parse().unwrap(), 0))
                .await
                .unwrap();
            peer.send_to(
                b"strict",
                SocketAddr::new(ip.parse().unwrap(), socket.local_addr().unwrap().port()),
            )
            .await
            .unwrap();
            let mut buf = [0; 16];
            let (n, _) = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                socket.recv_from(&mut buf),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(&buf[..n], b"strict");
        }
    }

    #[tokio::test]
    async fn tcp_binding_and_dns_dial_respect_family() {
        for family in [IpFamily::Auto, IpFamily::V4, IpFamily::V6] {
            let listener = bind_tcp(0, family).unwrap();
            if family == IpFamily::V6 {
                assert!(socket2::SockRef::from(&listener).only_v6().unwrap());
            }
            for host in ["127.0.0.1", "::1"] {
                if !family.allows(host.parse().unwrap())
                    || (host == "::1" && listener.local_addr().unwrap().is_ipv4())
                {
                    continue;
                }
                let stream =
                    connect_tcp_with_family(host, listener.local_addr().unwrap().port(), family)
                        .await
                        .unwrap();
                let (_, remote) =
                    tokio::time::timeout(std::time::Duration::from_secs(2), listener.accept())
                        .await
                        .unwrap()
                        .unwrap();
                assert_eq!(
                    canonical(remote).port(),
                    stream.local_addr().unwrap().port()
                );
            }
        }
        let error = connect_tcp_with_family("127.0.0.1", 1, IpFamily::V6)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AddrNotAvailable);
        assert!(error.to_string().contains("No DNS candidates"));
        assert_eq!(
            connect_tcp_with_family("::1", 1, IpFamily::V4)
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::AddrNotAvailable
        );
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
