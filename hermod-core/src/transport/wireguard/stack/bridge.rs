//! tokio 流适配（M4.2）：smoltcp TCP socket ↔ `AsyncRead` / `AsyncWrite`。
//!
//! 对应 DEVELOPMENT.md §3.4「桥接到异步」：数据面（单线程 poll smoltcp）与
//! 上层异步代码（HTTP / 文件 I/O / 进度渲染）通过**有界缓冲**交换字节，
//! 上层不会阻塞数据线程，内存有界（真实背压，R9 点名的易错点）。
//!
//! 方向：
//! - 读（socket → 用户）：有界 `mpsc`，`poll_recv` 可直接注册 waker；
//! - 写（用户 → socket）：自定义有界队列 + `AtomicWaker`（tokio mpsc 无 poll 版
//!   reserve，用 AtomicWaker 在队列腾出空间时唤醒写入方）。
//!
//! 数据线程每轮 poll 后调用 [`pump_socket`]。

use futures_util::task::AtomicWaker;
use std::collections::VecDeque;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::mpsc;

/// 桥接块大小。
const CHUNK: usize = 16 * 1024;
/// 读方向 channel 容量（块数）。
const READ_CAP: usize = 8;
/// 写方向队列容量（块数）。
const WRITE_CAP: usize = 8;

/// 写方向的有界队列（用户 → 数据面）。
struct SendQueue {
    queue: Mutex<VecDeque<Vec<u8>>>,
    /// 队列有空间时唤醒用户侧写入方。
    space: AtomicWaker,
    closed: AtomicBool,
}

impl SendQueue {
    fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            space: AtomicWaker::new(),
            closed: AtomicBool::new(false),
        }
    }

    /// 尝试压入一块；返回是否成功（队列满返回 false）。
    fn try_push(&self, data: Vec<u8>) -> bool {
        let mut q = self.queue.lock().unwrap();
        if q.len() >= WRITE_CAP {
            return false;
        }
        q.push_back(data);
        true
    }

    /// 数据面取走所有块（测试用）。
    #[cfg(test)]
    fn drain(&self) -> Vec<Vec<u8>> {
        let mut q = self.queue.lock().unwrap();
        let out: Vec<Vec<u8>> = q.drain(..).collect();
        drop(q);
        // 腾出空间：唤醒可能阻塞的写入方
        self.space.wake();
        out
    }

    /// 数据面逐条取（取不到返回 None），腾出空间时唤醒写入方。
    fn try_pop(&self) -> Option<Vec<u8>> {
        let mut q = self.queue.lock().unwrap();
        let out = q.pop_front();
        drop(q);
        if out.is_some() {
            self.space.wake();
        }
        out
    }

    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.space.wake();
    }
}

impl BridgeEnds {
    /// 关闭桥接两端：drop recv_tx 让用户侧读到 EOF，并关闭写队列。
    fn close(&mut self) {
        self.recv_tx.take();
        self.send.close();
    }
}

/// 用户侧桥接流（实现 `AsyncRead` + `AsyncWrite`）。
pub struct BridgeStream {
    recv: mpsc::Receiver<Vec<u8>>,
    send: Arc<SendQueue>,
    read_buf: Vec<u8>,
    read_pos: usize,
}

/// 数据面侧持有的两端。
pub struct BridgeEnds {
    pub recv_tx: Option<mpsc::Sender<Vec<u8>>>,
    send: Arc<SendQueue>,
    pending_recv: Vec<u8>,
    pending_send: Vec<u8>,
}

impl BridgeStream {
    /// 创建桥接的两端。
    pub fn new() -> (BridgeStream, BridgeEnds) {
        let (recv_tx, recv_rx) = mpsc::channel(READ_CAP);
        let send = Arc::new(SendQueue::new());
        let stream = BridgeStream {
            recv: recv_rx,
            send: send.clone(),
            read_buf: Vec::new(),
            read_pos: 0,
        };
        let ends = BridgeEnds {
            recv_tx: Some(recv_tx),
            send,
            pending_recv: Vec::new(),
            pending_send: Vec::new(),
        };
        (stream, ends)
    }
}

impl Default for BridgeStream {
    fn default() -> Self {
        Self::new().0
    }
}

impl AsyncRead for BridgeStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.read_pos < self.read_buf.len() {
            let n = (self.read_buf.len() - self.read_pos).min(buf.remaining());
            let start = self.read_pos;
            buf.put_slice(&self.read_buf[start..start + n]);
            self.read_pos += n;
            return Poll::Ready(Ok(()));
        }
        match self.recv.poll_recv(cx) {
            Poll::Ready(Some(chunk)) => {
                let n = chunk.len().min(buf.remaining());
                buf.put_slice(&chunk[..n]);
                if n < chunk.len() {
                    self.read_buf = chunk;
                    self.read_pos = n;
                } else {
                    self.read_buf.clear();
                    self.read_pos = 0;
                }
                Poll::Ready(Ok(()))
            }
            Poll::Ready(None) => Poll::Ready(Ok(())), // EOF
            Poll::Pending => Poll::Pending,
        }
    }
}

impl AsyncWrite for BridgeStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.send.closed.load(Ordering::SeqCst) {
            return Poll::Ready(Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed")));
        }
        if self.send.try_push(data.to_vec()) {
            return Poll::Ready(Ok(data.len()));
        }
        // 队列满：注册 waker，等待数据面 drain 后唤醒
        self.send.space.register(cx.waker());
        // 注册后再试一次，避免与 drain 的 wake 竞态丢唤醒
        if self.send.closed.load(Ordering::SeqCst) {
            return Poll::Ready(Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed")));
        }
        if self.send.try_push(data.to_vec()) {
            return Poll::Ready(Ok(data.len()));
        }
        Poll::Pending
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        // 数据面每轮都会 drain；无需额外动作。
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

/// 数据线程每轮 poll 后调用：在 smoltcp socket 与缓冲之间搬字节。
pub fn pump_socket(socket: &mut smoltcp::socket::tcp::Socket<'_>, ends: &mut BridgeEnds) {
    // 连接已彻底关闭（对端 FIN 或 RST）：关闭桥接两端，
    // 让应用侧的 read 立刻返回 EOF、write 返回 BrokenPipe，
    // 而不是一直挂起到上层超时。
    if socket.state() == smoltcp::socket::tcp::State::Closed {
        ends.close();
        return;
    }
    // ---- socket → 用户 ----
    let Some(recv_tx) = ends.recv_tx.as_ref() else {
        return;
    };
    if !ends.pending_recv.is_empty() {
        let chunk = std::mem::take(&mut ends.pending_recv);
        match recv_tx.try_send(chunk) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(v)) => {
                ends.pending_recv = v;
                return;
            }
            Err(mpsc::error::TrySendError::Closed(_)) => return,
        }
    }
    while socket.may_recv() {
        let mut buf = [0u8; CHUNK];
        match socket.recv_slice(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let chunk = buf[..n].to_vec();
                match recv_tx.try_send(chunk) {
                    Ok(()) => {}
                    Err(mpsc::error::TrySendError::Full(v)) => {
                        ends.pending_recv = v;
                        return;
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => return,
                }
            }
            Err(_) => break,
        }
    }

    // ---- 用户 → socket ----
    // 逐条取：socket 还不能发送（例如 TCP 仍在 SynSent）时，数据**留在队列里**，
    // 等连接建立后下一轮再发（早先无条件 drain 会在握手期间丢数据）。
    loop {
        if !ends.pending_send.is_empty() {
            if !socket.may_send() {
                break;
            }
            match socket.send_slice(&ends.pending_send) {
                Ok(0) => break,
                Ok(n) => {
                    ends.pending_send.drain(..n);
                }
                Err(_) => break,
            }
            if !ends.pending_send.is_empty() {
                break; // socket 缓冲满，下一轮继续
            }
        }
        if !socket.may_send() {
            break;
        }
        match ends.send.try_pop() {
            Some(chunk) => ends.pending_send = chunk,
            None => break,
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// 用户写 → 数据面侧读到；数据面侧发 → 用户读到。
    #[tokio::test]
    async fn stream_and_ends_roundtrip() {
        let (mut stream, ends) = BridgeStream::new();

        stream.write_all(b"hello").await.unwrap();
        stream.flush().await.unwrap();
        // 数据面通过 drain 取走
        let chunks = ends.send.drain();
        assert_eq!(chunks.concat(), b"hello");

        ends.recv_tx.as_ref().unwrap().send(b"world".to_vec()).await.unwrap();
        let mut buf = [0u8; 5];
        stream.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"world");
    }

    /// channel 关闭后读到 EOF（Ok(0)）。
    #[tokio::test]
    async fn eof_after_close() {
        let (mut stream, mut ends) = BridgeStream::new();
        ends.recv_tx.take();
        let mut buf = [0u8; 8];
        let n = stream.read(&mut buf).await.unwrap();
        assert_eq!(n, 0);
    }

    /// 端到端：两个 smoltcp 栈 + 桥接流，A 写 → B 读。
    #[tokio::test]
    async fn bridged_streams_transfer_between_stacks() {
        use crate::transport::wireguard::stack::device::IP_MTU;
        use crate::transport::wireguard::stack::driver::{
            TunnelStack, OVERLAY_RX, OVERLAY_TX,
        };
        use smoltcp::wire::{IpAddress, IpListenEndpoint, Ipv4Address};

        let ip = |a: std::net::Ipv4Addr| {
            let o = a.octets();
            IpAddress::Ipv4(Ipv4Address::new(o[0], o[1], o[2], o[3]))
        };
        let mut a = TunnelStack::new(OVERLAY_TX, 1, IP_MTU);
        let mut b = TunnelStack::new(OVERLAY_RX, 2, IP_MTU);

        let srv_h = b.add_tcp(TunnelStack::new_tcp_socket());
        b.listen(
            srv_h,
            IpListenEndpoint {
                addr: Some(ip(OVERLAY_RX)),
                port: 7000,
            },
        )
        .unwrap();
        let cli_h = a.add_tcp(TunnelStack::new_tcp_socket());
        a.connect(cli_h, (ip(OVERLAY_RX), 7000), 49152).unwrap();

        let pump = |a: &mut TunnelStack, b: &mut TunnelStack| {
            while let Some(p) = a.pop_inner() {
                b.push_inner(p);
            }
            while let Some(p) = b.pop_inner() {
                a.push_inner(p);
            }
            a.poll();
            b.poll();
        };

        // 先建立 TCP 连接
        for _ in 0..20 {
            pump(&mut a, &mut b);
            if a.tcp(cli_h).may_send() && b.tcp(srv_h).may_send() {
                break;
            }
        }
        assert!(a.tcp(cli_h).may_send() && b.tcp(srv_h).may_send());

        let (mut a_stream, mut a_ends) = BridgeStream::new();
        let (mut b_stream, mut b_ends) = BridgeStream::new();

        // A 写数据进自己的 send 队列
        a_stream.write_all(b"hello bridge").await.unwrap();

        // 驱动若干轮：桥接 pump + 栈交换
        for _ in 0..40 {
            pump_socket(a.tcp(cli_h), &mut a_ends);
            pump_socket(b.tcp(srv_h), &mut b_ends);
            pump(&mut a, &mut b);
        }

        // B 从自己的 recv 队列读
        let mut buf = [0u8; 12];
        b_stream.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"hello bridge");
    }
}
