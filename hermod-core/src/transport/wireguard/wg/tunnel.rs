//! WireGuard 会话（M3.2）：boringtun `Tunn` 的薄封装。
//!
//! 对应 DEVELOPMENT.md §3.3 与指南 M3.2。把 boringtun 的「输入缓冲 → 输出缓冲」
//! API 包装成三类事件：[`Outcome::Network`]（发到网络）、[`Outcome::Tunnel`]
//! （交给内层栈）、[`Outcome::Error`]（丢弃）。
//!
//! boringtun 的 `Tunn` 是**同步**状态机（方法调用时完成加解密），因此本封装也是
//! 同步的；由会话层（`session.rs`）的 async 循环驱动它。

use boringtun::noise::errors::WireGuardError;
use boringtun::noise::{Tunn, TunnResult};
use boringtun::x25519::{PublicKey, StaticSecret};
use std::net::IpAddr;

/// 单个 WG 包的最大缓冲（WG 包通常 < 1500 字节，留足余量）。
const BUF_SIZE: usize = 4096;

/// 处理一个 WG 包后的结果。
#[derive(Debug)]
pub enum Outcome {
    /// 不需要动作（握手进行中 / 空包）。
    None,
    /// 要发到网络的 WG 包。
    Network(Vec<u8>),
    /// 解密出的内层 IP 包（交给 smoltcp）。
    Tunnel(Vec<u8>),
    /// 处理失败（包被丢弃）。
    Error(WireGuardError),
}

/// WireGuard 会话（单对 peer、单会话）。
pub struct WireGuardSession {
    tunn: Tunn,
}

impl WireGuardSession {
    /// 创建会话。
    ///
    /// - `local`：本端临时私钥（`keys::KeyPair::secret()`）
    /// - `peer`：对端公钥（来自对端加密 bundle）
    /// - `psk`：由 K 派生的预共享密钥（`crypto::kdf::derive_wg_psk`）
    /// - `keepalive`：持久保活秒数（DEVELOPMENT.md §3.3 建议 25）
    /// - `index`：本会话唯一索引
    pub fn new(
        local: StaticSecret,
        peer: PublicKey,
        psk: [u8; 32],
        keepalive: Option<u16>,
        index: u32,
    ) -> Self {
        Self {
            tunn: Tunn::new(local, peer, Some(psk), keepalive, index, None),
        }
    }

    /// 发起握手，返回要发到网络的字节（无需发起时返回 `None`）。
    pub fn initiate_handshake(&mut self) -> Option<Vec<u8>> {
        let mut buf = vec![0u8; BUF_SIZE];
        match self.tunn.format_handshake_initiation(&mut buf, false) {
            TunnResult::WriteToNetwork(p) => Some(p.to_vec()),
            _ => None,
        }
    }

    /// 处理收到的 WG 包（握手发起/响应或数据）。
    pub fn handle_packet(&mut self, src: Option<IpAddr>, packet: &[u8]) -> Outcome {
        let mut buf = vec![0u8; BUF_SIZE];
        match self.tunn.decapsulate(src, packet, &mut buf) {
            TunnResult::Done => Outcome::None,
            TunnResult::Err(e) => Outcome::Error(e),
            TunnResult::WriteToNetwork(p) => Outcome::Network(p.to_vec()),
            TunnResult::WriteToTunnelV4(p, _) | TunnResult::WriteToTunnelV6(p, _) => {
                Outcome::Tunnel(p.to_vec())
            }
        }
    }

    /// 把内层 IP 包封装成 WG 包；返回要发到网络的字节。
    pub fn encapsulate(&mut self, packet: &[u8]) -> Option<Vec<u8>> {
        let mut buf = vec![0u8; BUF_SIZE];
        match self.tunn.encapsulate(packet, &mut buf) {
            TunnResult::WriteToNetwork(p) => Some(p.to_vec()),
            _ => None,
        }
    }

    /// 排空 boringtun 内部队列（例如握手完成后待发的包），返回要发到网络的字节。
    pub fn poll_queued(&mut self) -> Option<Vec<u8>> {
        let mut buf = vec![0u8; BUF_SIZE];
        match self.tunn.decapsulate(None, &[], &mut buf) {
            TunnResult::WriteToNetwork(p) => Some(p.to_vec()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::wireguard::wg::keys::KeyPair;
    use rand_core::UnwrapErr;

    const PSK: [u8; 32] = [0x5a; 32];

    fn keypair() -> KeyPair {
        KeyPair::generate(&mut UnwrapErr(getrandom::SysRng))
    }

    /// 建一对会话（可选不同的 PSK，用于测试 PSK 绑定）。
    fn make_pair(psk_a: [u8; 32], psk_b: [u8; 32]) -> (WireGuardSession, WireGuardSession) {
        let a = keypair();
        let b = keypair();
        let sa = WireGuardSession::new(a.secret(), b.public(), psk_a, Some(25), 1);
        let sb = WireGuardSession::new(b.secret(), a.public(), psk_b, Some(25), 2);
        (sa, sb)
    }

    /// 构造一个合法的 IPv4 包（boringtun 解密后会校验 IP 版本/长度）。
    fn ipv4_packet(payload: &[u8]) -> Vec<u8> {
        let total_len = 20 + payload.len();
        let mut p = vec![0u8; total_len];
        p[0] = 0x45; // IPv4, IHL=5
        p[2] = (total_len >> 8) as u8;
        p[3] = total_len as u8;
        p[8] = 64; // TTL
        p[9] = 17; // UDP
        p[12..16].copy_from_slice(&[10, 66, 66, 1]); // src（覆盖网 tx）
        p[16..20].copy_from_slice(&[10, 66, 66, 2]); // dst（覆盖网 rx）
        p[20..].copy_from_slice(payload);
        p
    }

    /// 完整握手 + 数据往返。
    #[test]
    fn handshake_and_data_roundtrip() {
        let (mut a, mut b) = make_pair(PSK, PSK);

        let init = a.initiate_handshake().expect("handshake initiation");
        let resp = match b.handle_packet(None, &init) {
            Outcome::Network(p) => p,
            other => panic!("expected handshake response, got {other:?}"),
        };
        // A 处理响应：握手完成（Done 或排空队列）
        let _ = a.handle_packet(None, &resp);
        let _ = a.poll_queued();

        let inner = ipv4_packet(b"hello wireguard");
        let data = a.encapsulate(&inner).expect("encapsulate data");
        match b.handle_packet(None, &data) {
            Outcome::Tunnel(p) => assert_eq!(p, inner),
            other => panic!("expected tunnel packet, got {other:?}"),
        }
    }

    /// PSK 不一致 → 握手失败（B 的响应无法被 A 接受，或直接报错）。
    #[test]
    fn wrong_psk_fails() {
        let (mut a, mut b) = make_pair(PSK, [0x99; 32]);
        let init = a.initiate_handshake().expect("handshake initiation");
        match b.handle_packet(None, &init) {
            Outcome::Error(_) => {}
            Outcome::Network(resp) => match a.handle_packet(None, &resp) {
                Outcome::Error(_) => {}
                Outcome::None => panic!("wrong PSK must not complete handshake"),
                other => panic!("unexpected: {other:?}"),
            },
            other => panic!("unexpected: {other:?}"),
        }
    }
}
