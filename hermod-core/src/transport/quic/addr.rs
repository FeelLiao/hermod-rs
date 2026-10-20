//! QUIC 的 IPv6 候选地址发现（独立实现）。
//!
//! 不复用 WireGuard 的 `addr` 模块：QUIC 只关心自己需要的东西。
//! 规则与项目整体一致：全局单播 → ULA → 链路本地；排除回环/组播/未指定/
//! IPv4-mapped；链路本地不参与跨主机候选（bundle 不带 scope id）。

use std::net::Ipv6Addr;

/// 候选类别，`Ord` 顺序即尝试顺序。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AddrClass {
    /// 全局单播。
    GlobalUnicast,
    /// ULA（fc00::/7）。
    Ula,
    /// 链路本地（fe80::/10）。
    LinkLocal,
}

fn is_ipv4_mapped(addr: &Ipv6Addr) -> bool {
    let o = addr.octets();
    o[..10] == [0u8; 10] && o[10] == 0xff && o[11] == 0xff
}

/// 分类；非候选返回 `None`。
pub fn classify(addr: &Ipv6Addr) -> Option<AddrClass> {
    if addr.is_loopback() || addr.is_multicast() || addr.is_unspecified() || is_ipv4_mapped(addr) {
        return None;
    }
    if addr.is_unique_local() {
        Some(AddrClass::Ula)
    } else if addr.is_unicast_link_local() {
        Some(AddrClass::LinkLocal)
    } else {
        Some(AddrClass::GlobalUnicast)
    }
}

/// 过滤 + 排序候选。
pub fn rank_candidates(mut addrs: Vec<Ipv6Addr>) -> Vec<Ipv6Addr> {
    addrs.retain(|a| classify(a).is_some());
    addrs.sort_by_key(|a| classify(a).expect("retained candidate must classify"));
    addrs
}

/// 枚举本机非回环 IPv6（排除链路本地，因为 bundle 无法携带 scope id）。
pub fn enumerate_local_ipv6() -> Vec<Ipv6Addr> {
    let Ok(ifaces) = if_addrs::get_if_addrs() else {
        return Vec::new();
    };
    let mut addrs = Vec::new();
    for iface in ifaces {
        if let std::net::IpAddr::V6(v6) = iface.addr.ip()
            && classify(&v6).is_some_and(|c| c != AddrClass::LinkLocal)
        {
            addrs.push(v6);
        }
    }
    rank_candidates(addrs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v6(s: &str) -> Ipv6Addr {
        s.parse().unwrap()
    }

    #[test]
    fn classify_variants() {
        assert_eq!(classify(&v6("2001:db8::1")), Some(AddrClass::GlobalUnicast));
        assert_eq!(classify(&v6("fd00::1")), Some(AddrClass::Ula));
        assert_eq!(classify(&v6("fe80::1")), Some(AddrClass::LinkLocal));
        assert_eq!(classify(&v6("::1")), None);
        assert_eq!(classify(&v6("ff02::1")), None);
        assert_eq!(classify(&v6("::")), None);
        assert_eq!(classify(&v6("::ffff:192.0.2.1")), None);
    }

    #[test]
    fn rank_filters_and_orders() {
        let input = vec![
            v6("fe80::1"),
            v6("2001:db8::1"),
            v6("::1"),
            v6("fd00::1"),
            v6("ff02::1"),
        ];
        assert_eq!(
            rank_candidates(input),
            vec![v6("2001:db8::1"), v6("fd00::1"), v6("fe80::1")]
        );
    }
}
