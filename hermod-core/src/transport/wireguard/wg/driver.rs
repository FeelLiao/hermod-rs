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
use std::net::{IpAddr, SocketAddr, SocketAddrV6};
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

/// 批量收包结果：(可选内层包, 需要回发的控制包列表)。
pub type RecvBatch = (Option<Vec<u8>>, Vec<(Vec<u8>, SocketAddrV6)>);

/// 一个 WireGuard 隧道会话（由调用方在 async 任务里驱动）。
pub struct WireGuardTunnel {
    session: WireGuardSession,
    udp: UdpSocket,
    endpoint: PeerEndpoint,
    /// boringtun 是否已有可用会话（能成功封装/解封装数据）。
    session_ok: bool,
    /// 复用收包缓冲（避免每包分配 64 KiB）。
    recv_buf: Vec<u8>,
}

impl WireGuardTunnel {
    /// 组合会话状态机、UDP socket 与对端端点。
    pub fn new(session: WireGuardSession, udp: UdpSocket, endpoint: PeerEndpoint) -> Self {
        Self {
            session,
            udp,
            endpoint,
            session_ok: false,
            recv_buf: vec![0u8; RECV_BUF],
        }
    }

    /// boringtun 会话是否已可用（收到过数据或成功封装过）。
    pub fn session_ok(&self) -> bool {
        self.session_ok
    }

    /// 对全部候选**同时**发握手发起包（§3.3）。
    pub async fn start_handshake(&mut self) -> Result<(), SessionError> {
        if let Some(pkt) = self.session.initiate_handshake() {
            let cands = self.endpoint.candidates().to_vec();
            crate::log_info!("发送 WireGuard 握手发起，目标 {} 个候选: {:?}", cands.len(), cands);
            for cand in cands {
                self.udp.send_to(&pkt, cand).await?;
            }
        }
        Ok(())
    }

    /// 重发握手（超时重试用）。已确定端点时只发该端点，否则继续广播候选。
    pub async fn retransmit_handshake(&mut self) -> Result<(), SessionError> {
        if let Some(pkt) = self.session.force_initiate_handshake() {
            if let Some(cur) = self.endpoint.current() {
                crate::log_debug!("重发握手 → 当前端点 {cur}");
                self.udp.send_to(&pkt, cur).await?;
            } else {
                let cands = self.endpoint.candidates().to_vec();
                crate::log_debug!("重发握手 → {} 个候选", cands.len());
                for cand in cands {
                    self.udp.send_to(&pkt, cand).await?;
                }
            }
        }
        Ok(())
    }

    /// 排空 boringtun 内部队列并把包发出去（握手完成前 encapsulate 会把内层包
    /// 排队；必须用空 datagram 反复 decapsulate 才能把它们放出来）。
    pub async fn flush_queued(&mut self) -> Result<(), SessionError> {
        while let Some(pkt) = self.session.poll_queued() {
            let to = self
                .endpoint
                .current()
                .or_else(|| self.endpoint.candidates().first().copied())
                .ok_or(SessionError::NoPeer)?;
            crate::log_debug!("排空 boringtun 队列：发送 {}B → {to}", pkt.len());
            self.udp.send_to(&pkt, to).await?;
        }
        Ok(())
    }

    /// 非阻塞地收一包（供驱动批量 drain）。返回 (内层包, 需回发的控制包)。
    pub fn try_recv_once(&mut self) -> Option<RecvBatch> {
        let (n, src) = self.udp.try_recv_from(&mut self.recv_buf).ok()?;
        let SocketAddr::V6(src6) = src else {
            return Some((None, Vec::new()));
        };
        let src_ip = IpAddr::V6(*src6.ip());
        match self.session.handle_packet(Some(src_ip), &self.recv_buf[..n]) {
            Outcome::Network(pkt) => {
                self.endpoint.roam(src6);
                Some((None, vec![(pkt, src6)]))
            }
            Outcome::Tunnel(inner) => {
                self.session_ok = true;
                self.endpoint.roam(src6);
                Some((Some(inner), Vec::new()))
            }
            Outcome::None => {
                self.endpoint.roam(src6);
                Some((None, Vec::new()))
            }
            Outcome::Error(e) => {
                crate::log_warn!("WG 解封装失败（{n}B，来自 {src6}）: {e:?}");
                Some((None, Vec::new()))
            }
        }
    }

    /// 直接把原始字节发给指定端点（批量 drain 后回发控制包用）。
    pub async fn send_raw(&mut self, pkt: &[u8], to: SocketAddrV6) -> Result<(), SessionError> {
        self.udp.send_to(pkt, to).await?;
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
        let (n, src) = self.udp.recv_from(&mut self.recv_buf).await?;
        let SocketAddr::V6(src6) = src else {
            // IPV6_V6ONLY 下不应收到 IPv4；防御性忽略。
            return Ok(None);
        };
        let src_ip = IpAddr::V6(*src6.ip());
        match self.session.handle_packet(Some(src_ip), &self.recv_buf[..n]) {
            Outcome::Network(pkt) => {
                // 握手响应 / keepalive 等：记录来源端点并回发。
                crate::log_debug!("收到 WG 控制包（{}B），回发 {}B，端点→{src6}", n, pkt.len());
                self.endpoint.roam(src6);
                self.udp.send_to(&pkt, src6).await?;
                Ok(None)
            }
            Outcome::Tunnel(inner) => {
                self.session_ok = true;
                self.endpoint.roam(src6);
                crate::log_debug!("收到 WG 数据包（{n}B）→ 内层 {}B", inner.len());
                Ok(Some(inner))
            }
            Outcome::None => {
                // Done：握手完成 / keepalive，成功处理。
                self.endpoint.roam(src6);
                Ok(None)
            }
            Outcome::Error(e) => {
                crate::log_warn!("WG 解封装失败（{n}B，来自 {src6}）: {e:?}");
                Ok(None)
            }
        }
    }

    /// 把内层 IP 包封装后发给对端（优先已确定的端点，否则首个候选）。
    pub async fn send_tunnel(&mut self, inner: &[u8]) -> Result<(), SessionError> {
        match self.session.encapsulate_len(inner) {
            Some(len) => {
                // 只有 WireGuard 数据包（type=4）才说明会话可用；
                // 无会话时 encapsulate 返回的是握手发起（type=1），不能算建立。
                let is_data = {
                    let pkt = self.session.scratch(len);
                    pkt.len() >= 4 && u32::from_le_bytes([pkt[0], pkt[1], pkt[2], pkt[3]]) == 4
                };
                if is_data {
                    self.session_ok = true;
                }
                let to = self
                    .endpoint
                    .current()
                    .or_else(|| self.endpoint.candidates().first().copied())
                    .ok_or(SessionError::NoPeer)?;
                crate::log_debug!("发送内层 {}B → WG {len}B → {to}", inner.len());
                self.udp.send_to(self.session.scratch(len), to).await?;
            }
            None => {
                crate::log_debug!("WG 会话未就绪，丢弃内层 {}B", inner.len());
            }
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
