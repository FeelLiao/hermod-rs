//! 对端地址发现（M2.5）：本地 IPv6 候选的过滤、排序与 server 观测地址合并。
//!
//! 对应 DEVELOPMENT.md §3.5：
//! - **只枚举 IPv6**，不枚举 IPv4（D7）；
//! - 候选顺序（双方一致）：全局单播 → ULA → 链路本地；
//! - 排除回环 / 组播 / 未指定 / IPv4-mapped；
//! - server 观测地址**仅作提示**，且仅当它是合法 IPv6 候选时才采纳。
//!
//! 本模块目前只做**纯逻辑**（可单测）；真实网卡枚举（`enumerate_local_ipv6`）
//! 是 stub，等引入 `if-addrs` 或平台 API 后再填（M2.5 提示）。

use std::net::Ipv6Addr;

/// IPv6 地址候选类别，`Ord` 顺序即尝试顺序（§3.5）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AddrClass {
    /// 全局单播：优先。
    GlobalUnicast,
    /// ULA（fc00::/7）：次之。
    Ula,
    /// 链路本地（fe80::/10）：最后（同链路场景）。
    LinkLocal,
}

/// 判断 IPv4-mapped 地址（`::ffff:a.b.c.d`，前缀 `::ffff:0:0/96`）。
/// （`Ipv6Addr::is_ipv4_mapped` 在本工具链上还不稳定，手写等价判断。）
fn is_ipv4_mapped(addr: &Ipv6Addr) -> bool {
    let o = addr.octets();
    o[..10] == [0u8; 10] && o[10] == 0xff && o[11] == 0xff
}

/// 把地址归为候选类别；非候选（回环/组播/未指定/IPv4-mapped）返回 `None`。
pub fn classify(addr: &Ipv6Addr) -> Option<AddrClass> {
    // 排除：回环 ::1、组播 ff00::/8、未指定 ::、IPv4-mapped ::ffff:a.b.c.d
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

/// 过滤 + 排序候选：全局单播 → ULA → 链路本地（§3.5，双方一致）。
pub fn rank_candidates(mut addrs: Vec<Ipv6Addr>) -> Vec<Ipv6Addr> {
    addrs.retain(|a| classify(a).is_some());
    // retain 后必有类别，unwrap 安全
    addrs.sort_by_key(|a| classify(a).expect("retained candidate must classify"));
    addrs
}

/// 把 server 观测地址并入候选列表：仅当它是合法 IPv6 候选且尚未在列表中时追加
/// （提示，§3.5）。IPv4-mapped 地址（经 IPv4 连 server 的观测）会被 [`classify`] 拒绝。
pub fn merge_server_hint(addrs: &mut Vec<Ipv6Addr>, hint: Ipv6Addr) {
    if classify(&hint).is_some() && !addrs.contains(&hint) {
        addrs.push(hint);
    }
}

/// 枚举本机非回环 IPv6 地址（M2.5，`if-addrs` 实现）。
///
/// 排除回环、组播、未指定、v4-mapped；链路本地不纳入候选（bundle 无法携带
/// scope id，跨主机不可达）。结果交给 [`rank_candidates`] 排序。
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

    /// 分类：合法三类 + 各种排除项。
    #[test]
    fn classify_variants() {
        assert_eq!(classify(&v6("2001:db8::1")), Some(AddrClass::GlobalUnicast));
        assert_eq!(
            classify(&v6("2606:4700::1111")),
            Some(AddrClass::GlobalUnicast)
        );
        assert_eq!(classify(&v6("fd00::1")), Some(AddrClass::Ula));
        assert_eq!(classify(&v6("fc00::1")), Some(AddrClass::Ula));
        assert_eq!(classify(&v6("fe80::1")), Some(AddrClass::LinkLocal));
        // 排除项
        assert_eq!(classify(&v6("::1")), None); // 回环
        assert_eq!(classify(&v6("ff02::1")), None); // 组播
        assert_eq!(classify(&v6("::")), None); // 未指定
        assert_eq!(classify(&v6("::ffff:192.0.2.1")), None); // IPv4-mapped
    }

    /// 排序：全局 → ULA → 链路本地；非法项被过滤。
    #[test]
    fn rank_orders_by_class() {
        let input = vec![
            v6("fe80::1"),
            v6("2001:db8::1"),
            v6("::1"), // 回环 → 过滤
            v6("fd00::1"),
            v6("ff02::1"), // 组播 → 过滤
        ];
        assert_eq!(
            rank_candidates(input),
            vec![v6("2001:db8::1"), v6("fd00::1"), v6("fe80::1")]
        );
    }

    /// server 观测地址：合法 IPv6 追加；重复不追加；IPv4-mapped 拒绝。
    #[test]
    fn merge_hint_rules() {
        let mut addrs = vec![v6("2001:db8::1")];
        merge_server_hint(&mut addrs, v6("2001:db8::2"));
        assert_eq!(addrs.len(), 2);
        // 重复的不再加
        merge_server_hint(&mut addrs, v6("2001:db8::1"));
        assert_eq!(addrs.len(), 2);
        // IPv4-mapped（经 IPv4 连 server 的观测）→ 拒绝
        merge_server_hint(&mut addrs, v6("::ffff:192.0.2.1"));
        assert_eq!(addrs.len(), 2);
    }
}
