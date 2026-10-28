//! WireGuard 传输实现（M3.4 集成）：把 UDP + boringtun + smoltcp + 桥接
//! 组装成 [`Transport`] / [`TransportConnection`]。
//!
//! 生命周期（DEVELOPMENT.md §3.2 步骤 5–7）：
//! 1. [`WireGuardTransport::prepare`]：绑 UDP、生成临时 X25519、枚举本机 IPv4/IPv6，
//!    返回加密 bundle 的明文（由上层用 CPace 密钥 AEAD 加密后经会合层交换）；
//! 2. [`WireGuardTransport::establish`]：解析对端 bundle、建会话、起驱动任务，
//!    返回 [`WgConnection`]；
//! 3. tx 调 `listen`、rx 调 `dial`，拿到的 `BridgeStream` 直接跑 [crate::transport::http]。

use crate::crypto::cpace::Role;
use crate::transport::iface::{AbortHandle, Transport, TransportConnection};
use crate::transport::wireguard::session::{BUNDLE_VERSION, EndpointBundle};
use crate::transport::wireguard::stack::bridge::{BridgeStream, pump_socket};
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
/// 主动 rekey 间隔：boringtun 的 REKEY_AFTER_TIME 是 120s、REJECT_AFTER_TIME 是
/// 180s，而 `Tunn` 不暴露 `update_timers`，不主动换 key 的话大文件传两分钟就会断。
const REKEY_INTERVAL: Duration = Duration::from_secs(100);

/// WireGuard 传输错误。
#[derive(Debug, thiserror::Error)]
pub enum WgError {
    #[error("udp/io: {0}")]
    Io(#[from] std::io::Error),
    #[error("peer bundle: {0}")]
    Bundle(#[from] serde_json::Error),
    #[error("peer bundle version mismatch: got {got}, expected {expected}")]
    VersionMismatch { got: u8, expected: u8 },
    #[error("peer credential fingerprint does not match this session token")]
    CredentialMismatch,
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
    /// server 观测到的对端出口地址（信令层 `ObservedAddrs.public_ipv6`）。
    /// Go 版把它作为 server-reflexive 候选放在最前；这里同语义。
    server_hint: Option<std::net::Ipv6Addr>,
    /// 内层 IP MTU（默认 1400，可 `--mtu` 覆盖）。
    mtu: usize,
    congestion: smoltcp::socket::tcp::CongestionControl,
    /// UDP 绑定地址（内部测试用；默认 `[::]:0`）。
    udp_bind: Option<std::net::SocketAddr>,
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
            server_hint: None,
            mtu: crate::transport::wireguard::stack::device::IP_MTU,
            congestion: smoltcp::socket::tcp::CongestionControl::Cubic,
            udp_bind: None,
        }
    }

    /// 注入信令服务器观测到的本端出口 IPv6 地址（Go 版的 server-reflexive）。
    ///
    /// 上层在 `allocate`/`join` 拿到 `ObservedAddrs` 后、[`prepare`](Transport::prepare)
    /// 前调用。只有它本身是合法 IPv6 候选时才生效；本地枚举仍是权威来源。
    pub fn set_server_hint(&mut self, hint: Option<std::net::Ipv6Addr>) {
        self.server_hint = hint;
    }

    /// 设置内层 IP MTU（默认 1420）。
    pub fn set_congestion(&mut self, value: smoltcp::socket::tcp::CongestionControl) {
        self.congestion = value;
    }

    pub fn set_mtu(&mut self, mtu: usize) {
        self.mtu = mtu;
    }

    /// 设置 UDP 绑定地址（内部测试用；默认 `[::]:0`）。
    pub fn set_udp_bind(&mut self, addr: Option<std::net::SocketAddr>) {
        self.udp_bind = addr;
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

    fn set_credential_fingerprint(&mut self, fingerprint: [u8; 32]) {
        self.fingerprint = fingerprint;
    }

    async fn prepare(&mut self) -> Result<Vec<u8>, WgError> {
        let udp = match self.udp_bind {
            Some(addr) => udp::bind_at(addr).await?,
            None => udp::bind(0).await?,
        };
        let udp_port = udp.local_addr()?.port();
        let mut rng = UnwrapErr(getrandom::SysRng);
        let keypair = KeyPair::generate(&mut rng);
        let mut candidates = crate::transport::wireguard::addr::enumerate_local_ipv6();
        // server 观测地址只当提示：仅采纳合法 IPv6，且放到候选最前
        // （对应 Go 版 CandidatesV6 里 public endpoint 在 local 之前）。
        if let Some(hint) = self.server_hint
            && crate::transport::wireguard::addr::classify(&hint).is_some()
        {
            candidates.retain(|a| *a != hint);
            candidates.insert(0, hint);
        }

        let (candidates, ipv4_candidates) =
            crate::transport::network::candidates(udp.local_addr()?, candidates);

        // 凭证指纹由会话层通过 set_credential_fingerprint 填入。
        let fingerprint = self.fingerprint;

        crate::log_info!(
            udp_port,
            http_port = self.http_port,
            inner_mtu = self.mtu,
            outer_mtu = self.mtu + 80,
            candidates = ?candidates,
            ipv4_candidates = ?ipv4_candidates,
            "transport prepared"
        );
        let bundle = EndpointBundle {
            version: BUNDLE_VERSION,
            wg_public_key: keypair.public_bytes(),
            udp_port,
            http_port: self.http_port,
            ipv6_candidates: candidates.clone(),
            ipv4_candidates,
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
        if peer.credential_fingerprint != self.fingerprint {
            return Err(WgError::CredentialMismatch);
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
        let addresses = crate::transport::network::peer_addresses(
            &peer.ipv6_candidates,
            &peer.ipv4_candidates,
            peer.udp_port,
            udp.local_addr()?,
        )?;
        let endpoint = PeerEndpoint::from_addrs(addresses);
        let tunnel = WireGuardTunnel::new(session, udp, endpoint);
        let mut stack = TunnelStack::new(overlay, seed, self.mtu);
        stack.set_congestion(self.congestion);

        crate::log_info!(
            role = ?role,
            peer_candidates = ?peer.ipv6_candidates,
            peer_udp_port = peer.udp_port,
            peer_http = peer.http_port,
            mtu = self.mtu,
            "establishing transport"
        );
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
    /// 优雅关闭：把发送队列排空后再让驱动退出。
    Shutdown { reply: oneshot::Sender<()> },
    /// 立即中止：RST 掉所有桥接 socket 并退出（用于 Ctrl+C 取消传输）。
    Abort,
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

    fn abort_handle(&self) -> AbortHandle {
        let cmd = self.cmd.clone();
        AbortHandle::new(move || {
            let _ = cmd.try_send(Command::Abort);
        })
    }

    async fn shutdown(&mut self) -> Result<(), WgError> {
        // 等驱动把缓冲里的数据全部送达对端后再返回，避免丢掉 POST /done 的裁决。
        let (reply, rx) = oneshot::channel();
        self.cmd
            .send(Command::Shutdown { reply })
            .await
            .map_err(|_| WgError::Closed)?;
        rx.await.map_err(|_| WgError::Closed)?;
        Ok(())
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
    // 文件异步打开和桌面调度可能慢于接收端拨号；握手前先准备 TCP 监听。
    let (stream, ends) = BridgeStream::new();
    let h = stack.add_tcp(TunnelStack::new_tcp_socket());
    let local = if role == Role::Sender {
        OVERLAY_TX
    } else {
        OVERLAY_RX
    };
    if let Err(error) = stack.listen(
        h,
        smoltcp::wire::IpListenEndpoint {
            addr: Some(ipv4_addr(local)),
            port: local_http,
        },
    ) {
        crate::log_warn!(%error, "could not prepare HTTP listener");
        return;
    }
    let mut listener = Some(stream);
    let mut bridged = vec![(h, ends)];
    let mut ticker = tokio::time::interval(TICK);
    let mut last_keepalive = Instant::now();
    /// rx 的超时兜底：3 秒仍未建立才自己发起。
    const RX_FALLBACK: Duration = Duration::from_secs(3);

    // 固定角色（DESIGN §3.3 的退路）：tx 主动发起、rx 只响应；rx 仅在超时后
    // 才发起（应对地址变化）。同时发起会让 boringtun 的握手竞态不稳定。
    let is_sender = role == Role::Sender;
    crate::log_info!(
        role = ?role,
        local_http,
        peer_http,
        mtu = stack.mtu(),
        "transport driver started"
    );
    if is_sender && tunnel.start_handshake().await.is_err() {
        crate::log_warn!("initial handshake send failed, driver exiting");
        return;
    }
    let mut last_handshake = Instant::now();
    let mut last_rekey = Instant::now();
    let mut iter = 0u64;
    let mut inner_rx = 0u64;
    let mut inner_tx = 0u64;
    // ---- TCP/WG 统计 ----
    let mut inner_rx_bytes = 0u64;
    let mut inner_tx_bytes = 0u64;
    let mut last_tx_seq: Option<u32> = None;
    let mut last_ack: Option<u32> = None;
    let mut retx = 0u64;
    let mut dup_ack = 0u64;
    let mut last_rx_at = Instant::now();
    let mut prev_rx_bytes = 0u64;
    let mut prev_tx_bytes = 0u64;
    let mut prev_stats_at = Instant::now();

    loop {
        tokio::select! {
            r = tunnel.recv() => {
                match r {
                    Ok(Some(inner)) => {
                        inner_rx += 1;
                        inner_rx_bytes += inner.len() as u64;
                        last_rx_at = Instant::now();
                        if let Some((_, ack, false, true, _)) = tcp_fields(&inner) {
                            if last_ack == Some(ack) {
                                dup_ack += 1;
                            } else {
                                last_ack = Some(ack);
                            }
                        }
                        crate::log_debug!("inner packet #{inner_rx} ({}B) -> smoltcp", inner.len());
                        stack.push_inner(inner);
                    }
                    Ok(None) => {}
                    Err(e) => {
                        crate::log_warn!("UDP receive failed: {e}");
                        break;
                    }
                }
            }
            cmd = cmd_rx.recv() => {
                let Some(cmd) = cmd else {
                    // 用户侧连接已关闭：给驱动一个排空窗口，把缓冲里还没发出的
                    // 数据（例如 POST /done 的裁决）真正写进 WG，再退出。
                    for _ in 0..10 {
                        stack.poll();
                        for (h, ends) in bridged.iter_mut() {
                            pump_socket(stack.tcp(*h), ends);
                        }
                        while let Some(pkt) = stack.pop_inner() {
                            let _ = tunnel.send_tunnel(&pkt).await;
                        }
                        tokio::time::sleep(TICK).await;
                    }
                    break;
                };
                match cmd {
                    Command::Listen { reply } => {
                        crate::log_info!(port = local_http, "listening");
                        let _ = reply.send(listener.take().ok_or(WgError::Closed));
                    }
                    Command::Dial { reply } => {
                        crate::log_info!(peer_http, "dialing");
                        let (stream, ends) = BridgeStream::new();
                        let h = stack.add_tcp(TunnelStack::new_tcp_socket());
                        let peer = if role == Role::Sender { OVERLAY_RX } else { OVERLAY_TX };
                        let _ = stack.connect(h, (ipv4_addr(peer), peer_http), DIAL_LOCAL_PORT);
                        bridged.push((h, ends));
                        let _ = reply.send(Ok(stream));
                    }
                    Command::Abort => {
                        // 立即取消：RST 掉所有桥接 socket（对端会立刻收到并结束），
                        // 同时关闭本端桥接，让本地的 serve/fetch 任务也立即返回。
                        for (h, ends) in bridged.iter_mut() {
                            stack.tcp(*h).abort();
                            ends.close();
                        }
                        stack.poll();
                        while let Some(pkt) = stack.pop_inner() {
                            let _ = tunnel.send_tunnel(&pkt).await;
                        }
                        crate::log_info!("transport aborted (RST sent)");
                        break;
                    }
                    Command::Shutdown { reply } => {
                        // 排空：把 BridgeStream 缓冲与 smoltcp 发送队列都写出去，
                        // 直到全部被对端 ACK（send_queue == 0），再退出，避免丢掉
                        // 最后的裁决/尾数据。
                        for _ in 0..200 {
                            stack.poll();
                            for (h, ends) in bridged.iter_mut() {
                                pump_socket(stack.tcp(*h), ends);
                            }
                            let mut sent_any = false;
                            while let Some(pkt) = stack.pop_inner() {
                                let _ = tunnel.send_tunnel(&pkt).await;
                                sent_any = true;
                            }
                            let busy = sent_any
                                || bridged
                                    .iter()
                                    .any(|(h, _)| stack.tcp(*h).send_queue() > 0);
                            if !busy {
                                break;
                            }
                            tokio::time::sleep(TICK).await;
                        }
                        let _ = reply.send(());
                        break;
                    }
                }
            }
            _ = ticker.tick() => {}
        }

        iter += 1;
        // 批量收包：一次 drain 最多 128 个 UDP 包，再统一 poll 一次 smoltcp，
        // 避免「每包一次完整 poll」在高包速率下追不上内核、导致成批丢包。
        let mut batch = 0usize;
        while let Some((inner, responses)) = tunnel.try_recv_once() {
            batch += 1;
            if let Some(inner) = inner {
                inner_rx += 1;
                inner_rx_bytes += inner.len() as u64;
                last_rx_at = Instant::now();
                if let Some((_, ack, false, true, _)) = tcp_fields(&inner) {
                    if last_ack == Some(ack) {
                        dup_ack += 1;
                    } else {
                        last_ack = Some(ack);
                    }
                }
                stack.push_inner(inner);
            }
            for (pkt, to) in responses {
                let _ = tunnel.send_raw(&pkt, to).await;
            }
            if batch >= 512 {
                break;
            }
        }
        if batch > 64 || iter.is_multiple_of(200) {
            crate::log_debug!("batch recv: {batch} packets this round");
        }
        // 驱动 smoltcp，并搬运每个桥接 socket 的字节
        stack.poll();
        for (h, ends) in bridged.iter_mut() {
            pump_socket(stack.tcp(*h), ends);
        }
        while let Some(pkt) = stack.pop_inner() {
            inner_tx += 1;
            inner_tx_bytes += pkt.len() as u64;
            if let Some((seq, _, true, _, plen)) = tcp_fields(&pkt) {
                if let Some(expect) = last_tx_seq
                    && seq != expect
                {
                    retx += 1;
                }
                last_tx_seq = Some(seq.wrapping_add(plen as u32));
            }
            let _ = tunnel.send_tunnel(&pkt).await;
        }
        // 把 boringtun 排队的内层包放出来（握手完成前发送的包会排在内部队列）
        if let Err(e) = tunnel.flush_queued().await {
            crate::log_warn!("failed to flush WG queue: {e}");
        }
        if iter.is_multiple_of(40) {
            let now = Instant::now();
            let dt = now.duration_since(prev_stats_at).as_secs_f64().max(0.001);
            let rx_mib_s = (inner_rx_bytes - prev_rx_bytes) as f64 / dt / 1_048_576.0;
            let tx_mib_s = (inner_tx_bytes - prev_tx_bytes) as f64 / dt / 1_048_576.0;
            prev_rx_bytes = inner_rx_bytes;
            prev_tx_bytes = inner_tx_bytes;
            prev_stats_at = now;

            let mut snd_q = 0usize;
            let mut snd_cap = 0usize;
            let mut rcv_q = 0usize;
            let mut rcv_cap = 0usize;
            for (h, _) in bridged.iter() {
                let s = stack.tcp(*h);
                snd_q += s.send_queue();
                snd_cap += s.send_capacity();
                rcv_q += s.recv_queue();
                rcv_cap += s.recv_capacity();
            }
            let mut bridge = crate::transport::wireguard::stack::bridge::BridgeStats::default();
            for (_, ends) in bridged.iter() {
                let st = ends.stats();
                bridge.pushed_bytes += st.pushed_bytes;
                bridge.push_full += st.push_full;
                bridge.popped_bytes += st.popped_bytes;
                bridge.delivered_bytes += st.delivered_bytes;
                bridge.recv_full += st.recv_full;
            }
            let wg_idle_s = now.duration_since(last_rx_at).as_secs_f64();
            crate::log_debug!(
                role = ?role,
                rx_mib_s,
                tx_mib_s,
                snd_q,
                snd_cap,
                rcv_q,
                rcv_cap,
                retx,
                dup_ack,
                bridge_pushed = bridge.pushed_bytes,
                bridge_push_full = bridge.push_full,
                bridge_delivered = bridge.delivered_bytes,
                bridge_recv_full = bridge.recv_full,
                wg_idle_s,
                "tcp stats"
            );
            if wg_idle_s > 2.0 {
                crate::log_debug!(wg_idle_s, "no WG delivery to TCP");
            }
        }
        if iter.is_multiple_of(200) {
            crate::log_info!(
                rx_mib = inner_rx_bytes / 1_048_576,
                tx_mib = inner_tx_bytes / 1_048_576,
                retx,
                dup_ack,
                "tcp transfer summary"
            );
        }

        // 定时器：握手重试 / 保活（boringtun 不暴露 update_timers）
        if !tunnel.session_ok() {
            let retry = if is_sender {
                HANDSHAKE_RETRY
            } else {
                RX_FALLBACK
            };
            if last_handshake.elapsed() >= retry {
                crate::log_debug!("session not ready, retransmit handshake (role={role:?})");
                let _ = tunnel.retransmit_handshake().await;
                last_handshake = Instant::now();
            }
        } else {
            // 会话可用：定期强制 rekey，避免 boringtun 会话在 180s 后过期
            if is_sender && last_rekey.elapsed() >= REKEY_INTERVAL {
                crate::log_info!("WireGuard rekey (avoid session expiry)");
                let _ = tunnel.retransmit_handshake().await;
                last_rekey = Instant::now();
            }
            if last_keepalive.elapsed() >= Duration::from_secs(KEEPALIVE_SECS) {
                crate::log_debug!("sending WireGuard keepalive");
                let _ = tunnel.send_keepalive().await;
                last_keepalive = Instant::now();
            }
        }
    }
    crate::log_info!(
        "transport driver exited (role={role:?}, inner_rx={inner_rx} inner_tx={inner_tx})"
    );
}

/// 从内层 IPv4 包解析 TCP 关键字段（仅统计用）：
/// `(seq, ack, 有载荷, 是 ACK, 载荷长度)`。
fn tcp_fields(pkt: &[u8]) -> Option<(u32, u32, bool, bool, usize)> {
    let ip = smoltcp::wire::Ipv4Packet::new_checked(pkt).ok()?;
    if ip.next_header() != smoltcp::wire::IpProtocol::Tcp {
        return None;
    }
    let tcp = smoltcp::wire::TcpPacket::new_checked(ip.payload()).ok()?;
    let payload_len = tcp.payload().len();
    Some((
        tcp.seq_number().0 as u32,
        tcp.ack_number().0 as u32,
        payload_len > 0,
        tcp.ack(),
        payload_len,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// server 观测地址（server-reflexive）放在候选最前。
    #[tokio::test]
    async fn prepare_puts_server_hint_first() {
        let mut t = WireGuardTransport::new();
        let hint: std::net::Ipv6Addr = "fd00::abcd".parse().unwrap();
        t.set_server_hint(Some(hint));
        let bytes = t.prepare().await.unwrap();
        let b = EndpointBundle::decode(&bytes).unwrap();
        assert_eq!(b.ipv6_candidates.first(), Some(&hint));
    }

    /// v4-mapped 观测地址（经 IPv4 连 server）必须被忽略（D7）。
    #[tokio::test]
    async fn prepare_ignores_v4_mapped_hint() {
        let mut t = WireGuardTransport::new();
        let v4mapped: std::net::Ipv6Addr = "::ffff:1.2.3.4".parse().unwrap();
        t.set_server_hint(Some(v4mapped));
        let bytes = t.prepare().await.unwrap();
        let b = EndpointBundle::decode(&bytes).unwrap();
        assert!(!b.ipv6_candidates.contains(&v4mapped));
    }

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
        let _stream = tokio::time::timeout(std::time::Duration::from_secs(1), a_conn.listen())
            .await
            .expect("listen 应立即返回")
            .unwrap();
    }

    /// 会话层 token 指纹不一致时，establish 必须在建连前拒绝。
    #[tokio::test]
    async fn credential_mismatch_is_rejected() {
        let mut a = WireGuardTransport::new();
        let mut b = WireGuardTransport::new();
        a.set_credential_fingerprint([0x01; 32]);
        b.set_credential_fingerprint([0x02; 32]);
        let _a_ep = a.prepare().await.unwrap();
        let b_ep = b.prepare().await.unwrap();
        let psk = crate::crypto::kdf::derive_wg_psk(&[0x5b; 32], 7);
        let err = a.establish(Role::Sender, &psk, &b_ep).await.err();
        assert!(matches!(err, Some(WgError::CredentialMismatch)));
    }
}
