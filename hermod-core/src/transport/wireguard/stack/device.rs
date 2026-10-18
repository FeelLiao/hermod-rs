//! smoltcp 设备桥接（M4.1 / M3.4）：内存队列假设备。
//!
//! 实现 smoltcp 的 [`Device`] trait（`receive` / `transmit`）：
//! - WireGuard 解封装出的内层 IP 包进入 RX 队列（[`QueueDevice::push_rx`]）；
//! - smoltcp 要发的包进入 TX 队列（[`QueueDevice::pop_tx`]），由驱动线程取走、
//!   交给 WireGuard 封装。
//!
//! 用内存队列（而非真实网卡）让整个数据面可单元测试，也是 DEVELOPMENT.md 风险 R9
//! 建议的「内存假设备」做法。

use smoltcp::phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken};
use smoltcp::time::Instant;
use std::collections::VecDeque;

/// 内层 IP MTU（DEVELOPMENT.md §3.3：1500 − 40(IPv6) − 8(UDP) − 32(WG) = 1420）。
pub const IP_MTU: usize = 1420;

/// 基于内存队列的 smoltcp 设备。
pub struct QueueDevice {
    rx: VecDeque<Vec<u8>>,
    tx: VecDeque<Vec<u8>>,
    mtu: usize,
}

impl QueueDevice {
    /// 创建指定 MTU 的队列设备。
    pub fn new(mtu: usize) -> Self {
        Self {
            rx: VecDeque::new(),
            tx: VecDeque::new(),
            mtu,
        }
    }

    /// 注入一个收到的内层包（来自 WireGuard 解封装）。
    pub fn push_rx(&mut self, pkt: Vec<u8>) {
        self.rx.push_back(pkt);
    }

    /// 取出一个待发出去的内层包（交给 WireGuard 封装）。
    pub fn pop_tx(&mut self) -> Option<Vec<u8>> {
        self.tx.pop_front()
    }

    /// 待发队列长度（背压/诊断用）。
    pub fn pending_tx(&self) -> usize {
        self.tx.len()
    }
}

impl Device for QueueDevice {
    type RxToken<'a> = RxBuf;
    type TxToken<'a> = TxBuf<'a>;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let pkt = self.rx.pop_front()?;
        Some((RxBuf(pkt), TxBuf { queue: &mut self.tx }))
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        Some(TxBuf {
            queue: &mut self.tx,
        })
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ip;
        caps.max_transmission_unit = self.mtu;
        caps
    }
}

/// 接收令牌：持有一个已收到的包，交给 smoltcp 解析。
pub struct RxBuf(pub Vec<u8>);

impl RxToken for RxBuf {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        f(&self.0)
    }
}

/// 发送令牌：smoltcp 写入要发的包，构造完成后进 TX 队列。
pub struct TxBuf<'a> {
    queue: &'a mut VecDeque<Vec<u8>>,
}

impl TxToken for TxBuf<'_> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut buf = vec![0u8; len];
        let r = f(&mut buf);
        self.queue.push_back(buf);
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RX：注入的包能被 receive.consume 读回。
    #[test]
    fn receive_returns_injected_packet() {
        let mut dev = QueueDevice::new(IP_MTU);
        dev.push_rx(vec![1, 2, 3, 4]);
        let (rx, _tx) = dev.receive(Instant::from_millis(0)).expect("rx token");
        let got = rx.consume(|b| b.to_vec());
        assert_eq!(got, vec![1, 2, 3, 4]);
        // 队列已空
        assert!(dev.receive(Instant::from_millis(0)).is_none());
    }

    /// TX：smoltcp 写入的包能通过 pop_tx 取出。
    #[test]
    fn transmit_packet_lands_in_tx_queue() {
        let mut dev = QueueDevice::new(IP_MTU);
        let tx = dev.transmit(Instant::from_millis(0)).expect("tx token");
        tx.consume(3, |buf| {
            buf.copy_from_slice(&[9, 8, 7]);
        });
        assert_eq!(dev.pending_tx(), 1);
        assert_eq!(dev.pop_tx(), Some(vec![9, 8, 7]));
        assert_eq!(dev.pop_tx(), None);
    }
}
