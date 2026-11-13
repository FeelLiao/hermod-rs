//! 使用公开传输契约和内存流验证编排行为，不访问 crate 私有实现。

use hermod_core::{
    options::{LinkProfile, TransferTuning as ConnectionTuning},
    orchestrate::{PayloadSource, ReceiveOptions, ServeOptions, receive, send_file, serve_file},
    transfer::{Kind, TransferOffer},
    transport::{AbortHandle, TransportConnection},
};
use std::{
    io,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, ReadBuf},
    sync::mpsc,
};

struct TrackedStream {
    inner: tokio::io::DuplexStream,
    active: Option<Arc<AtomicUsize>>,
}

impl Drop for TrackedStream {
    fn drop(&mut self) {
        if let Some(active) = &self.active {
            active.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

impl AsyncRead for TrackedStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for TrackedStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

struct MemoryConnection {
    incoming: mpsc::Receiver<TrackedStream>,
    outgoing: mpsc::Sender<TrackedStream>,
    tuning: ConnectionTuning,
    opened: usize,
    active: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
}

impl TransportConnection for MemoryConnection {
    type Error = io::Error;
    type Stream = TrackedStream;
    fn http_port(&self) -> u16 {
        0
    }
    fn peer_http_port(&self) -> u16 {
        0
    }
    async fn listen(&mut self) -> io::Result<Self::Stream> {
        self.incoming
            .recv()
            .await
            .ok_or_else(|| io::ErrorKind::BrokenPipe.into())
    }
    async fn dial(&mut self) -> io::Result<Self::Stream> {
        let (a, b) = tokio::io::duplex(8192);
        self.outgoing
            .send(TrackedStream {
                inner: b,
                active: None,
            })
            .await
            .map_err(|_| io::ErrorKind::BrokenPipe)?;
        // 控制流也计入总流数；数据流并发预算另加一条控制流。
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(active, Ordering::SeqCst);
        self.opened += 1;
        Ok(TrackedStream {
            inner: a,
            active: Some(self.active.clone()),
        })
    }
    fn abort_handle(&self) -> AbortHandle {
        AbortHandle::new(|| {})
    }
    async fn shutdown(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn transfer_tuning(&self) -> ConnectionTuning {
        self.tuning
    }
}

fn memory_pair(tx: ConnectionTuning, rx: ConnectionTuning) -> (MemoryConnection, MemoryConnection) {
    let (a, b) = mpsc::channel(16);
    let (c, d) = mpsc::channel(16);
    let connection = |incoming, outgoing, tuning| MemoryConnection {
        incoming,
        outgoing,
        tuning,
        opened: 0,
        active: Arc::new(AtomicUsize::new(0)),
        peak: Arc::new(AtomicUsize::new(0)),
    };
    (connection(b, c, tx), connection(d, a, rx))
}

#[tokio::test]
async fn different_chunks_interoperate_with_bounded_data_streams() {
    for streams in [1, 2, 8] {
        let tx_tuning = ConnectionTuning {
            streams: Some(1),
            chunk: 256 * 1024,
            prefetch: 4,
            ..Default::default()
        };
        let rx_tuning = ConnectionTuning {
            streams: Some(streams),
            chunk: 1024 * 1024 + 17,
            slice_size: 4 * 1024 * 1024 + 7,
            prefetch: 4,
            ..Default::default()
        };
        let (mut tx, mut rx) = memory_pair(tx_tuning, rx_tuning);
        let dir =
            std::env::temp_dir().join(format!("hermod-tuning-{}-{streams}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.bin");
        let data: Vec<u8> = (0..40 * 1024 * 1024 + 73)
            .map(|i| (i % 251) as u8)
            .collect();
        std::fs::write(&source, &data).unwrap();
        let offer = TransferOffer {
            kind: Kind::File,
            name: "result.bin".into(),
            size: data.len() as u64,
        };
        let out = dir.join("out");
        let (sent, received) = tokio::time::timeout(Duration::from_secs(30), async {
            tokio::join!(
                send_file(&mut tx, &source, None),
                receive(
                    &mut rx,
                    &out,
                    ReceiveOptions {
                        offer: Some(&offer),
                        profile: LinkProfile::Wan,
                        peer_multi_stream: true,
                        ..Default::default()
                    }
                )
            )
        })
        .await
        .expect("bounded scheduler must not deadlock");
        assert!(sent.unwrap().verified);
        let receipt = received.unwrap();
        assert_eq!(std::fs::read(&receipt.path).unwrap(), data);
        assert_eq!(rx.active.load(Ordering::SeqCst), 0);
        assert!(rx.peak.load(Ordering::SeqCst) <= if streams == 1 { 1 } else { streams + 1 });
        if streams > 1 {
            assert!(
                rx.opened > streams + 1,
                "must replenish rather than open all slices"
            );
            assert!(rx.peak.load(Ordering::SeqCst) > 2);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[tokio::test]
async fn nonseekable_sender_forces_single_even_with_stream_override() {
    let tuning = ConnectionTuning {
        streams: Some(8),
        ..Default::default()
    };
    let (mut tx, mut rx) = memory_pair(tuning, tuning);
    let size = 16 * 1024 * 1024;
    let dir = std::env::temp_dir().join(format!("hermod-nonseek-{}", std::process::id()));
    let offer = TransferOffer {
        kind: Kind::File,
        name: "stream.bin".into(),
        size,
    };
    let opts = ServeOptions {
        kind: offer.kind,
        name: offer.name.clone(),
        size,
        source: PayloadSource::Stream(Box::new(tokio::io::repeat(42).take(size))),
        progress: None,
    };
    let (sent, received) = tokio::time::timeout(Duration::from_secs(15), async {
        tokio::join!(
            serve_file(&mut tx, opts),
            receive(
                &mut rx,
                &dir,
                ReceiveOptions {
                    offer: Some(&offer),
                    profile: LinkProfile::Lan,
                    peer_multi_stream: false,
                    ..Default::default()
                }
            )
        )
    })
    .await
    .unwrap();
    assert!(sent.unwrap().verified);
    assert_eq!(received.unwrap().size, size);
    assert_eq!(rx.opened, 1);
    std::fs::remove_dir_all(dir).unwrap();
}
