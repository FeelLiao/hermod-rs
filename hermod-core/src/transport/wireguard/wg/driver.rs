//! WireGuard 会话驱动（M3.3）：UDP + Tunn + 端点。
//!
//! 把 boringtun 的同步状态机（[`WireGuardSession`]）与 tokio UDP socket 绑成
//! 一个可驱动的结构：
//! - [`start_handshake`](WireGuardTunnel::start_handshake)：对全部候选**同时**
//!   发握手（§3.3 同时发起，一个 RTT 完成开路 + 认证）；
//! - [`recv`](WireGuardTunnel::recv)：读一个 UDP 包，处理握手/数据，返回解密出的
//!   内层 IP 包（交给 smoltcp，M3.4）；
//! - [`send_tunnel`](WireGuardTunnel::send_tunnel)：把内层包封装后发给对端；
//! - 重传/保活：boringtun 不暴露定时器，由外层定时器调用
//!   [`retransmit_handshake`](WireGuardTunnel::retransmit_handshake) /
//!   [`send_keepalive`](WireGuardTunnel::send_keepalive)。

use crate::transport::wireguard::wg::endpoint::PeerEndpoint;
use crate::transport::wireguard::wg::tunnel::{Outcome, WireGuardSession};
use std::net::{IpAddr, SocketAddr};
use tokio::net::UdpSocket;

/// UDP 收包缓冲（最大 UDP 载荷 65535）。
const RECV_BUF: usize = 65536;

/// 会话驱动错误。
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("udp error: {0}")]
    Io(#[from] std::io::Error),
    #[error("no peer endpoint available")]
    NoPeer,
}

/// 一个 WireGuard 隧道会话（由调用方在 async 任务里驱动）。
pub struct WireGuardTunnel {
    session: WireGuardSession,
    udp: UdpSocket,
    endpoint: PeerEndpoint,
}

impl WireGuardTunnel {
    /// 组合会话状态机、UDP socket 与对端端点。
    pub fn new(session: WireGuardSession, udp: UdpSocket, endpoint: PeerEndpoint) -> Self {
        Self {
            session,
            udp,
            endpoint,
        }
    }

    /// 对全部候选**同时**发握手发起包（§3.3）。
    pub async fn start_handshake(&mut self) -> Result<(), SessionError> {
        if let Some(pkt) = self.session.initiate_handshake() {
            for cand in self.endpoint.candidates().to_vec() {
                self.udp.send_to(&pkt, cand).await?;
            }
        }
        Ok(())
    }

    /// 重发握手（超时重试用）。已确定端点时只发该端点，否则继续广播候选。
    pub async fn retransmit_handshake(&mut self) -> Result<(), SessionError> {
        if let Some(pkt) = self.session.force_initiate_handshake() {
            if let Some(cur) = self.endpoint.current() {
                self.udp.send_to(&pkt, cur).await?;
            } else {
                for cand in self.endpoint.candidates().to_vec() {
                    self.udp.send_to(&pkt, cand).await?;
                }
            }
        }
        Ok(())
    }

    /// 发保活包（空载荷）。仅会话已建立时有字节可发。
    pub async fn send_keepalive(&mut self) -> Result<(), SessionError> {
        if let Some(pkt) = self.session.send_keepalive() {
            let to = self
                .endpoint
                .current()
                .or_else(|| self.endpoint.candidates().first().copied())
                .ok_or(SessionError::NoPeer)?;
            self.udp.send_to(&pkt, to).await?;
        }
        Ok(())
    }

    /// 读一个 UDP 包并处理；返回解密出的内层 IP 包（若有）。
    ///
    /// 漫游（§3.3）：任何**成功**处理的报文（握手响应、数据、完成），其源地址
    /// 成为新的对端端点。认证失败的包（`Outcome::Error`）不更新端点。
    pub async fn recv(&mut self) -> Result<Option<Vec<u8>>, SessionError> {
        let mut buf = [0u8; RECV_BUF];
        let (n, src) = self.udp.recv_from(&mut buf).await?;
        let SocketAddr::V6(src6) = src else {
            // IPV6_V6ONLY 下不应收到 IPv4；防御性忽略。
            return Ok(None);
        };
        let src_ip = IpAddr::V6(*src6.ip());
        match self.session.handle_packet(Some(src_ip), &buf[..n]) {
            Outcome::Network(pkt) => {
                // 握手响应 / keepalive 等：记录来源端点并回发。
                self.endpoint.roam(src6);
                self.udp.send_to(&pkt, src6).await?;
                Ok(None)
            }
            Outcome::Tunnel(inner) => {
                self.endpoint.roam(src6);
                Ok(Some(inner))
            }
            Outcome::None => {
                // Done：握手完成 / keepalive，成功处理。
                self.endpoint.roam(src6);
                Ok(None)
            }
            Outcome::Error(_) => Ok(None),
        }
    }

    /// 把内层 IP 包封装后发给对端（优先已确定的端点，否则首个候选）。
    pub async fn send_tunnel(&mut self, inner: &[u8]) -> Result<(), SessionError> {
        if let Some(pkt) = self.session.encapsulate(inner) {
            let to = self
                .endpoint
                .current()
                .or_else(|| self.endpoint.candidates().first().copied())
                .ok_or(SessionError::NoPeer)?;
            self.udp.send_to(&pkt, to).await?;
        }
        Ok(())
    }

    /// 当前对端端点（未建立前为 `None`）。
    pub fn endpoint(&self) -> &PeerEndpoint {
        &self.endpoint
    }

    /// 是否已确定对端端点（收到过已认证的包）。
    pub fn is_established(&self) -> bool {
        self.endpoint.is_established()
    }

    /// 本端 UDP 端口。
    pub fn local_port(&self) -> std::io::Result<u16> {
        self.udp.local_addr().map(|a| a.port())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::wireguard::udp;
    use crate::transport::wireguard::wg::keys::KeyPair;
    use rand_core::UnwrapErr;
    use std::net::SocketAddrV6;

    /// 构造合法 IPv4 内层包。
    fn ipv4_packet(payload: &[u8]) -> Vec<u8> {
        let total_len = 20 + payload.len();
        let mut p = vec![0u8; total_len];
        p[0] = 0x45;
        p[2] = (total_len >> 8) as u8;
        p[3] = total_len as u8;
        p[8] = 64;
        p[9] = 17;
        p[12..16].copy_from_slice(&[10, 66, 66, 1]);
        p[16..20].copy_from_slice(&[10, 66, 66, 2]);
        p[20..].copy_from_slice(payload);
        p
    }

    /// 两个会话在 IPv6 回环上握手并传一个内层包（模拟真实 UDP 路径）。
    #[tokio::test]
    async fn handshake_and_data_over_loopback_udp() {
        let mut rng = UnwrapErr(getrandom::SysRng);
        let ka = KeyPair::generate(&mut rng);
        let kb = KeyPair::generate(&mut rng);
        let psk = [0x42u8; 32];

        let udp_a = udp::bind(0).await.unwrap();
        let udp_b = udp::bind(0).await.unwrap();
        let port_b = udp_b.local_addr().unwrap().port();
        let port_a = udp_a.local_addr().unwrap().port();

        let lo = std::net::Ipv6Addr::LOCALHOST;
        // 回环地址会被 PeerEndpoint::new 的候选过滤排除，测试用 from_addrs。
        let ep_a = PeerEndpoint::from_addrs(vec![SocketAddrV6::new(lo, port_b, 0, 0)]);
        let ep_b = PeerEndpoint::from_addrs(vec![SocketAddrV6::new(lo, port_a, 0, 0)]);

        let sa = WireGuardSession::new(ka.secret(), kb.public(), psk, Some(25), 1);
        let sb = WireGuardSession::new(kb.secret(), ka.public(), psk, Some(25), 2);
        let mut a = WireGuardTunnel::new(sa, udp_a, ep_a);
        let mut b = WireGuardTunnel::new(sb, udp_b, ep_b);

        // 握手：A 发起 → B 响应 → A 完成
        a.start_handshake().await.unwrap();
        let _ = b.recv().await.unwrap();
        let _ = a.recv().await.unwrap();
        assert!(a.is_established(), "A 应已确定对端端点");
        assert!(b.is_established(), "B 应已确定对端端点");

        // A → B 数据。注意：握手完成时 boringtun 会自动回一个 keepalive，
        // 所以循环 recv 直到拿到内层包（真实驱动循环也会一直 recv）。
        let inner = ipv4_packet(b"ping from a");
        a.send_tunnel(&inner).await.unwrap();
        let mut got = None;
        for _ in 0..10 {
            if let Some(p) = b.recv().await.unwrap() {
                got = Some(p);
                break;
            }
        }
        assert_eq!(got.expect("B 应收到内层包"), inner);
    }
}
