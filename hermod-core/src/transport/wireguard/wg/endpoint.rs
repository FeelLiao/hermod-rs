//! 对端端点选择与漫游（M3.2/M3.3）。
//!
//! 对应 DEVELOPMENT.md §3.3/§3.5：
//! - 候选地址按「全局单播 → ULA → 链路本地」排序（复用 [`addr`]）；
//! - 建连时对**全部候选同时**发握手（一个 RTT 同时开路 + 认证）；
//! - 漫游：任何成功解封装的 WG 报文的源地址 = 新的对端端点。

use crate::transport::wireguard::addr;
use std::net::{Ipv6Addr, SocketAddrV6};

/// 对端端点：候选列表 + 当前生效端点（漫游更新）。
#[derive(Debug, Clone, Default)]
pub struct PeerEndpoint {
    /// 候选地址（已按 §3.5 排序）。
    candidates: Vec<SocketAddrV6>,
    /// 当前生效的对端端点（收到第一个已认证包后确定；漫游时更新）。
    current: Option<SocketAddrV6>,
}

impl PeerEndpoint {
    /// 由对端 bundle 里的 IPv6 候选 + 对端 UDP 端口创建（候选按 §3.5 排序）。
    ///
    /// 链路本地地址的 scope id 不在 bundle 里（`Ipv6Addr` 无 scope），
    /// 这里填 0；同链路场景的 scope 需在绑定 socket 时处理（后续阶段）。
    pub fn new(addrs: Vec<Ipv6Addr>, port: u16) -> Self {
        let candidates = addr::rank_candidates(addrs)
            .into_iter()
            .map(|a| SocketAddrV6::new(a, port, 0, 0))
            .collect();
        Self {
            candidates,
            current: None,
        }
    }

    /// 直接用给定的套接字地址构造候选（不排序、不过滤；测试与内部使用）。
    pub fn from_addrs(candidates: Vec<SocketAddrV6>) -> Self {
        Self {
            candidates,
            current: None,
        }
    }

    /// 全部候选（按顺序；建连时对它们同时发握手）。
    pub fn candidates(&self) -> &[SocketAddrV6] {
        &self.candidates
    }

    /// 当前生效的端点。
    pub fn current(&self) -> Option<SocketAddrV6> {
        self.current
    }

    /// 是否已确定对端端点（至少收到过一个已认证的包）。
    pub fn is_established(&self) -> bool {
        self.current.is_some()
    }

    /// 漫游：成功解封装的报文源地址成为新端点（WireGuard 规则，§3.3）。
    pub fn roam(&mut self, src: SocketAddrV6) {
        self.current = Some(src);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v6(s: &str) -> Ipv6Addr {
        s.parse().unwrap()
    }

    /// 候选按「全局 → ULA → 链路本地」排序，并统一使用对端端口。
    #[test]
    fn candidates_are_ranked() {
        let ep = PeerEndpoint::new(
            vec![v6("fe80::1"), v6("2001:db8::1"), v6("fd00::1")],
            51820,
        );
        let got: Vec<_> = ep
            .candidates()
            .iter()
            .map(|a| (a.ip().to_string(), a.port()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("2001:db8::1".to_string(), 51820),
                ("fd00::1".to_string(), 51820),
                ("fe80::1".to_string(), 51820),
            ]
        );
    }

    /// 未收到包前 current 为空；roam 后确立，且可再次漫游更新。
    #[test]
    fn roaming_updates_current() {
        let mut ep = PeerEndpoint::new(vec![v6("2001:db8::1")], 51820);
        assert!(!ep.is_established());
        assert_eq!(ep.current(), None);

        let a = SocketAddrV6::new(v6("2001:db8::2"), 51820, 0, 0);
        ep.roam(a);
        assert!(ep.is_established());
        assert_eq!(ep.current(), Some(a));

        // 漫游到新地址（例如临时地址被弃用）
        let b = SocketAddrV6::new(v6("2001:db8::3"), 51820, 0, 0);
        ep.roam(b);
        assert_eq!(ep.current(), Some(b));
    }
}
