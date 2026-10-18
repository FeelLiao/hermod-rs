//! smoltcp 栈驱动（M4.1 / M3.4）：Interface + SocketSet + 队列设备。
//!
//! 把 WireGuard 的内层 IP 包与 smoltcp 的 TCP/IP 栈接起来：
//! - 驱动线程把 WG 解封装出的包 [`push_inner`](TunnelStack::push_inner) 注入；
//! - [`poll`](TunnelStack::poll) 后从 [`pop_inner`](TunnelStack::pop_inner) 取包，
//!   交给 WireGuard 封装。
//!
//! 覆盖网地址（DEVELOPMENT.md §3.3）：tx = 10.66.66.1，rx = 10.66.66.2。

use crate::transport::wireguard::stack::device::{QueueDevice, IP_MTU};
use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet};
use smoltcp::socket::tcp;
use smoltcp::time::Instant;
use smoltcp::wire::{HardwareAddress, IpAddress, IpCidr, Ipv4Address};
use std::net::Ipv4Addr;

/// 覆盖网地址（tx）。
pub const OVERLAY_TX: Ipv4Addr = Ipv4Addr::new(10, 66, 66, 1);
/// 覆盖网地址（rx）。
pub const OVERLAY_RX: Ipv4Addr = Ipv4Addr::new(10, 66, 66, 2);
/// 每方向 TCP socket 缓冲（DEVELOPMENT.md §3.4：8 MiB）。
pub const SOCKET_BUF: usize = 8 * 1024 * 1024;

/// 隧道内 smoltcp 栈（单线程驱动；不是线程安全的，由数据线程独占）。
pub struct TunnelStack {
    device: QueueDevice,
    iface: Interface,
    sockets: SocketSet<'static>,
    start: std::time::Instant,
}

impl TunnelStack {
    /// 创建栈并配置覆盖网地址（tx 或 rx）。
    pub fn new(overlay: Ipv4Addr, random_seed: u64) -> Self {
        let mut device = QueueDevice::new(IP_MTU);
        let mut config = Config::new(HardwareAddress::Ip);
        config.random_seed = random_seed;
        let mut iface = Interface::new(config, &mut device, Instant::from_millis(0));
        iface.update_ip_addrs(|addrs| {
            // 覆盖网是点对点链路：用 /30 子网（10.66.66.0/30 同时覆盖 .1 与 .2），
            // smoltcp 才能把包路由给对端；纯 /32 无路由会被丢弃。
            let o = overlay.octets();
            let _ = addrs.push(IpCidr::new(
                IpAddress::Ipv4(Ipv4Address::new(o[0], o[1], o[2], o[3])),
                30,
            ));
        });
        Self {
            device,
            iface,
            sockets: SocketSet::new(Vec::new()),
            start: std::time::Instant::now(),
        }
    }

    fn now(&self) -> Instant {
        Instant::from_millis(self.start.elapsed().as_millis() as i64)
    }

    /// 注入一个 WG 解封装出的内层包。
    pub fn push_inner(&mut self, pkt: Vec<u8>) {
        self.device.push_rx(pkt);
    }

    /// 取出一个待封装发出的内层包。
    pub fn pop_inner(&mut self) -> Option<Vec<u8>> {
        self.device.pop_tx()
    }

    /// poll 一次：处理 ingress / egress / socket 状态。
    pub fn poll(&mut self) {
        let now = self.now();
        self.iface
            .poll(now, &mut self.device, &mut self.sockets);
    }

    /// 把一个 TCP socket 加入栈，返回句柄。
    pub fn add_tcp(&mut self, socket: tcp::Socket<'static>) -> SocketHandle {
        self.sockets.add(socket)
    }

    /// 取 socket 的可变引用。
    pub fn tcp(&mut self, handle: SocketHandle) -> &mut tcp::Socket<'static> {
        self.sockets.get_mut(handle)
    }

    /// 让 socket 拨向 `remote`（smoltcp 0.14 的 connect 需要 interface 的
    /// `Context` 注册 waker，这里用字段级拆分借用来同时拿到 iface 与 socket）。
    pub fn connect(
        &mut self,
        handle: SocketHandle,
        remote: impl Into<smoltcp::wire::IpEndpoint>,
        local_port: u16,
    ) -> Result<(), tcp::ConnectError> {
        let cx = self.iface.context();
        self.sockets
            .get_mut::<tcp::Socket>(handle)
            .connect(cx, remote, local_port)
    }

    /// 让 socket 在 `endpoint` 上监听。
    pub fn listen(
        &mut self,
        handle: SocketHandle,
        endpoint: smoltcp::wire::IpListenEndpoint,
    ) -> Result<(), tcp::ListenError> {
        self.sockets
            .get_mut::<tcp::Socket>(handle)
            .listen(endpoint)
    }

    /// 新建 8 MiB 缓冲的 TCP socket（尚未加入栈）。
    pub fn new_tcp_socket() -> tcp::Socket<'static> {
        tcp::Socket::new(
            tcp::SocketBuffer::new(vec![0u8; SOCKET_BUF]),
            tcp::SocketBuffer::new(vec![0u8; SOCKET_BUF]),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smoltcp::wire::IpListenEndpoint;

    fn ipv4(a: Ipv4Addr) -> IpAddress {
        let o = a.octets();
        IpAddress::Ipv4(Ipv4Address::new(o[0], o[1], o[2], o[3]))
    }

    /// 双向泵一轮：交换双方 TX→RX，然后各自 poll，再泵一次。
    fn pump(a: &mut TunnelStack, b: &mut TunnelStack) {
        for _ in 0..2 {
            while let Some(pkt) = a.pop_inner() {
                b.push_inner(pkt);
            }
            while let Some(pkt) = b.pop_inner() {
                a.push_inner(pkt);
            }
            a.poll();
            b.poll();
        }
    }

    /// 两个栈经内存假设备建立 TCP 连接并传数据（smoltcp 栈验收）。
    #[test]
    fn tcp_connect_and_transfer_between_two_stacks() {
        let mut a = TunnelStack::new(OVERLAY_TX, 1);
        let mut b = TunnelStack::new(OVERLAY_RX, 2);

        // B 监听 10.66.66.2:7777
        let srv = TunnelStack::new_tcp_socket();
        let srv_h = b.add_tcp(srv);
        b.listen(
            srv_h,
            IpListenEndpoint {
                addr: Some(ipv4(OVERLAY_RX)),
                port: 7777,
            },
        )
        .expect("listen");

        // A 拨向 B（local port 不能为 0）
        let cli = TunnelStack::new_tcp_socket();
        let cli_h = a.add_tcp(cli);
        a.connect(cli_h, (ipv4(OVERLAY_RX), 7777), 49152)
            .expect("connect");

        // 泵几轮直到连接**建立**（注意 is_active 对 SynSent/SynReceived 也 true，
        // 必须用 may_send 判断 Established）
        for _ in 0..20 {
            pump(&mut a, &mut b);
            if a.tcp(cli_h).may_send() && b.tcp(srv_h).may_send() {
                break;
            }
        }
        assert!(a.tcp(cli_h).may_send(), "client 应建立连接");
        assert!(b.tcp(srv_h).may_send(), "server 应建立连接");

        // A 发数据 → B 收
        let payload = b"hello over smoltcp".to_vec();
        let sent = a.tcp(cli_h).send_slice(&payload).expect("send");
        assert_eq!(sent, payload.len());
        let mut got = Vec::new();
        for _ in 0..20 {
            pump(&mut a, &mut b);
            if b.tcp(srv_h).may_recv() {
                let mut buf = [0u8; 64];
                let n = b.tcp(srv_h).recv_slice(&mut buf).expect("recv");
                got.extend_from_slice(&buf[..n]);
                if got.len() >= payload.len() {
                    break;
                }
            }
        }
        assert_eq!(got, payload);
    }
}
