//! WireGuard 传输实现（M3.4 集成）：把 UDP + boringtun + smoltcp + 桥接
//! 组装成 [`Transport`] / [`TransportConnection`]。
//!
//! 生命周期（DEVELOPMENT.md §3.2 步骤 5–7）：
//! 1. [`WireGuardTransport::prepare`]：绑 UDP、生成临时 X25519、枚举本机 IPv6，
//!    返回加密 bundle 的明文（由上层用 CPace 密钥 AEAD 加密后经会合层交换）；
//! 2. [`WireGuardTransport::establish`]：解析对端 bundle、建会话、起驱动任务，
//!    返回 [`WgConnection`]；
//! 3. tx 调 `listen`、rx 调 `dial`，拿到的 `BridgeStream` 直接跑隧道内 HTTP。

use crate::crypto::cpace::Role;
use crate::transport::iface::{Transport, TransportConnection};
use crate::transport::wireguard::session::{BUNDLE_VERSION, EndpointBundle};
use crate::transport::wireguard::stack::bridge::{BridgeEnds, BridgeStream, pump_socket};
use crate::transport::wireguard::stack::driver::{OVERLAY_RX, OVERLAY_TX, TunnelStack};
use crate::transport::wireguard::udp;
use crate::transport::wireguard::wg::driver::WireGuardTunnel;
use crate::transport::wireguard::wg::endpoint::PeerEndpoint;
use crate::transport::wireguard::wg::keys::{KeyPair, parse_public_key};
use crate::transport::wireguard::wg::tunnel::WireGuardSession;
use rand_core::{Rng, UnwrapErr};
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};

/// 隧道内 HTTP 端口（tx 监听；rx 拨号）。进程内栈，端口无特权。
pub const HTTP_PORT: u16 = 80;
/// rx 拨号时的本地端口。
const DIAL_LOCAL_PORT: u16 = 49200;
/// WireGuard 持久保活（§3.3）。
const KEEPALIVE_SECS: u64 = 25;
/// 握手重试间隔。
const HANDSHAKE_RETRY: Duration = Duration::from_secs(1);
/// 驱动 poll 间隔。
const TICK: Duration = Duration::from_millis(50);

/// WireGuard 传输错误。
#[derive(Debug, thiserror::Error)]
pub enum WgError {
    #[error("udp/io: {0}")]
    Io(#[from] std::io::Error),
    #[error("peer bundle: {0}")]
    Bundle(#[from] serde_json::Error),
    #[error("peer bundle version mismatch: got {got}, expected {expected}")]
    VersionMismatch { got: u8, expected: u8 },
    #[error("transport not prepared")]
    NotPrepared,
    #[error("driver closed")]
    Closed,
}

/// WireGuard 传输：prepare 后持有 UDP socket / 密钥对，establish 后交给驱动。
pub struct WireGuardTransport {
    udp: Option<tokio::net::UdpSocket>,
    keypair: Option<KeyPair>,
    http_port: u16,
    candidates: Vec<std::net::Ipv6Addr>,
    fingerprint: [u8; 32],
    overlay: Ipv4Addr,
}

impl WireGuardTransport {
    /// 新建（尚未 prepare）。
    pub fn new() -> Self {
        Self {
            udp: None,
            keypair: None,
            http_port: HTTP_PORT,
            candidates: Vec::new(),
            fingerprint: [0u8; 32],
            overlay: OVERLAY_TX,
        }
    }
}

impl Default for WireGuardTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl Transport for WireGuardTransport {
    type Error = WgError;
    type Connection = WgConnection;

    async fn prepare(&mut self) -> Result<Vec<u8>, WgError> {
        let udp = udp::bind(0).await?;
        let udp_port = udp.local_addr()?.port();
        let mut rng = UnwrapErr(getrandom::SysRng);
        let keypair = KeyPair::generate(&mut rng);
        let candidates = crate::transport::wireguard::addr::enumerate_local_ipv6();

        // 一次性凭证指纹：这里用随机凭证的 SHA-256（真实 token 由 M4.4 生成）。
        let mut credential = [0u8; 32];
        rng.fill_bytes(&mut credential);
        let fingerprint: [u8; 32] = {
            use sha2::{Digest, Sha256};
            Sha256::digest(credential).into()
        };

        let bundle = EndpointBundle {
            version: BUNDLE_VERSION,
            wg_public_key: keypair.public_bytes(),
            udp_port,
            http_port: self.http_port,
            ipv6_candidates: candidates.clone(),
            credential_fingerprint: fingerprint,
        };
        self.udp = Some(udp);
        self.keypair = Some(keypair);
        self.candidates = candidates;
        self.fingerprint = fingerprint;
        Ok(bundle.encode()?)
    }

    async fn establish(
        &mut self,
        role: Role,
        psk: &[u8; 32],
        peer_endpoint: &[u8],
    ) -> Result<WgConnection, WgError> {
        let peer = EndpointBundle::decode(peer_endpoint)?;
        if peer.version != BUNDLE_VERSION {
            return Err(WgError::VersionMismatch {
                got: peer.version,
                expected: BUNDLE_VERSION,
            });
        }
        let udp = self.udp.take().ok_or(WgError::NotPrepared)?;
        let keypair = self.keypair.take().ok_or(WgError::NotPrepared)?;

        let overlay = if role == Role::Sender {
            OVERLAY_TX
        } else {
            OVERLAY_RX
        };
        self.overlay = overlay;

        // `psk` 已是上层用 `derive_wg_psk(K, channel_id)` 派生好的 WireGuard PSK，
        // 两端一致、从不过网；这里直接使用，不重复派生。
        let wg_psk = *psk;

        let mut rng = UnwrapErr(getrandom::SysRng);
        let seed = u64::from_le_bytes({
            let mut b = [0u8; 8];
            rng.fill_bytes(&mut b);
            b
        });
        let index = (seed as u32) & 0x00ff_ffff;

        let session = WireGuardSession::new(
            keypair.secret(),
            parse_public_key(peer.wg_public_key),
            wg_psk,
            Some(KEEPALIVE_SECS as u16),
            index,
        );
        let endpoint = PeerEndpoint::new(peer.ipv6_candidates.clone(), peer.udp_port);
        let tunnel = WireGuardTunnel::new(session, udp, endpoint);
        let stack = TunnelStack::new(overlay, seed);

        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        let local_http = self.http_port;
        let peer_http = peer.http_port;
        tokio::spawn(driver_loop(
            tunnel, stack, role, local_http, peer_http, cmd_rx,
        ));

        Ok(WgConnection {
            http_port: local_http,
            peer_http_port: peer_http,
            cmd: cmd_tx,
        })
    }
}

/// 驱动任务命令。
enum Command {
    Listen {
        reply: oneshot::Sender<Result<BridgeStream, WgError>>,
    },
    Dial {
        reply: oneshot::Sender<Result<BridgeStream, WgError>>,
    },
}

/// 已建立的 WireGuard 连接（上层用它 listen/dial）。
pub struct WgConnection {
    http_port: u16,
    peer_http_port: u16,
    cmd: mpsc::Sender<Command>,
}

impl TransportConnection for WgConnection {
    type Error = WgError;
    type Stream = BridgeStream;

    fn http_port(&self) -> u16 {
        self.http_port
    }

    fn peer_http_port(&self) -> u16 {
        self.peer_http_port
    }

    async fn listen(&mut self) -> Result<BridgeStream, WgError> {
        let (reply, rx) = oneshot::channel();
        self.cmd
            .send(Command::Listen { reply })
            .await
            .map_err(|_| WgError::Closed)?;
        rx.await.map_err(|_| WgError::Closed)?
    }

    async fn dial(&mut self) -> Result<BridgeStream, WgError> {
        let (reply, rx) = oneshot::channel();
        self.cmd
            .send(Command::Dial { reply })
            .await
            .map_err(|_| WgError::Closed)?;
        rx.await.map_err(|_| WgError::Closed)?
    }
}

fn ipv4_addr(a: Ipv4Addr) -> smoltcp::wire::IpAddress {
    let o = a.octets();
    smoltcp::wire::IpAddress::Ipv4(smoltcp::wire::Ipv4Address::new(o[0], o[1], o[2], o[3]))
}

/// 驱动循环：UDP ⇄ WireGuard ⇄ smoltcp ⇄ 桥接流。
async fn driver_loop(
    mut tunnel: WireGuardTunnel,
    mut stack: TunnelStack,
    role: Role,
    local_http: u16,
    peer_http: u16,
    mut cmd_rx: mpsc::Receiver<Command>,
) {
    let mut bridged: Vec<(smoltcp::iface::SocketHandle, BridgeEnds)> = Vec::new();
    let mut ticker = tokio::time::interval(TICK);
    let mut last_keepalive = Instant::now();
    /// rx 的超时兜底：3 秒仍未建立才自己发起。
    const RX_FALLBACK: Duration = Duration::from_secs(3);

    // 固定角色（DESIGN §3.3 的退路）：tx 主动发起、rx 只响应；rx 仅在超时后
    // 才发起（应对地址变化）。同时发起会让 boringtun 的握手竞态不稳定。
    let is_sender = role == Role::Sender;
    if is_sender && tunnel.start_handshake().await.is_err() {
        return;
    }
    let mut last_handshake = Instant::now();

    loop {
        tokio::select! {
            r = tunnel.recv() => {
                match r {
                    Ok(Some(inner)) => stack.push_inner(inner),
                    Ok(None) => {}
                    Err(_) => break,
                }
            }
            cmd = cmd_rx.recv() => {
                let Some(cmd) = cmd else { break };
                match cmd {
                    Command::Listen { reply } => {
                        let (stream, ends) = BridgeStream::new();
                        let h = stack.add_tcp(TunnelStack::new_tcp_socket());
                        let local = if role == Role::Sender { OVERLAY_TX } else { OVERLAY_RX };
                        let _ = stack.listen(
                            h,
                            smoltcp::wire::IpListenEndpoint { addr: Some(ipv4_addr(local)), port: local_http },
                        );
                        bridged.push((h, ends));
                        let _ = reply.send(Ok(stream));
                    }
                    Command::Dial { reply } => {
                        let (stream, ends) = BridgeStream::new();
                        let h = stack.add_tcp(TunnelStack::new_tcp_socket());
                        let peer = if role == Role::Sender { OVERLAY_RX } else { OVERLAY_TX };
                        let _ = stack.connect(h, (ipv4_addr(peer), peer_http), DIAL_LOCAL_PORT);
                        bridged.push((h, ends));
                        let _ = reply.send(Ok(stream));
                    }
                }
            }
            _ = ticker.tick() => {}
        }

        // 驱动 smoltcp，并搬运每个桥接 socket 的字节
        stack.poll();
        for (h, ends) in bridged.iter_mut() {
            pump_socket(stack.tcp(*h), ends);
        }
        while let Some(pkt) = stack.pop_inner() {
            let _ = tunnel.send_tunnel(&pkt).await;
        }

        // 定时器：握手重试 / 保活（boringtun 不暴露 update_timers）
        if !tunnel.is_established() {
            let retry = if is_sender { HANDSHAKE_RETRY } else { RX_FALLBACK };
            if last_handshake.elapsed() >= retry {
                let _ = tunnel.retransmit_handshake().await;
                last_handshake = Instant::now();
            }
        } else if last_keepalive.elapsed() >= Duration::from_secs(KEEPALIVE_SECS) {
            let _ = tunnel.send_keepalive().await;
            last_keepalive = Instant::now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// prepare 产出的 bundle 能被解析，且字段合理。
    #[tokio::test]
    async fn prepare_produces_valid_bundle() {
        let mut t = WireGuardTransport::new();
        let bytes = t.prepare().await.unwrap();
        let b = EndpointBundle::decode(&bytes).unwrap();
        assert_eq!(b.version, BUNDLE_VERSION);
        assert_eq!(b.http_port, HTTP_PORT);
        assert_ne!(b.udp_port, 0);
        assert_ne!(b.wg_public_key, [0u8; 32]);
    }

    /// 端到端：两个 WireGuardTransport 在本机（真实枚举的 IPv6，内核本地投递）
    /// 完成 WG 握手 + smoltcp TCP + 桥接流传输。
    #[tokio::test]
    async fn end_to_end_two_transports() {
        use std::time::Duration;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let mut tx = WireGuardTransport::new();
        let mut rx = WireGuardTransport::new();
        let tx_ep = tx.prepare().await.unwrap();
        let rx_ep = rx.prepare().await.unwrap();

        let k = [0x33u8; 32];
        let psk = crate::crypto::kdf::derive_wg_psk(&k, 7);

        let mut tx_conn = tx.establish(Role::Sender, &psk, &rx_ep).await.unwrap();
        let mut rx_conn = rx.establish(Role::Receiver, &psk, &tx_ep).await.unwrap();

        // tx 监听，rx 拨号（都是立即返回空闲流）
        let mut server = tx_conn.listen().await.unwrap();
        let mut client = rx_conn.dial().await.unwrap();

        // rx 写 → tx 读（内含 WG 握手 + TCP 建连，给足超时）
        client.write_all(b"hermod e2e").await.unwrap();
        let mut buf = [0u8; 10];
        tokio::time::timeout(Duration::from_secs(15), server.read_exact(&mut buf))
            .await
            .expect("超时：握手/建连未完成")
            .expect("读取失败");
        assert_eq!(&buf, b"hermod e2e");
    }

    /// establish 能消费 prepare 的产物并返回连接；对端 bundle 由另一个
    /// prepare 生成（模拟真实双方都先 prepare）。
    #[tokio::test]
    async fn establish_returns_connection() {
        let mut a = WireGuardTransport::new();
        let mut b = WireGuardTransport::new();
        let a_ep = a.prepare().await.unwrap();
        let b_ep = b.prepare().await.unwrap();
        let psk = crate::crypto::kdf::derive_wg_psk(&[0x5a; 32], 7);

        let mut a_conn = a.establish(Role::Sender, &psk, &b_ep).await.unwrap();
        let _b_conn = b.establish(Role::Receiver, &psk, &a_ep).await.unwrap();

        assert_eq!(a_conn.http_port(), HTTP_PORT);
        assert_eq!(a_conn.peer_http_port(), HTTP_PORT);

        // tx 监听；拿到的流是 BridgeStream（未握手前读会挂起，这不阻塞测试，
        // 因为 listen 立即返回空闲流）
        let _stream = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            a_conn.listen(),
        )
        .await
        .expect("listen 应立即返回")
        .unwrap();
    }
}
