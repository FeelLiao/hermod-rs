//! 单任务传输控制：进度、连接状态、取消和摘要，直接使用核心库而非解析 CLI 输出。

use hermod_core::transport::{AbortHandle, NetworkStatsHandle, TransportConnection};
use hermod_core::{
    config::Config,
    orchestrate,
    session::{self, SessionOptions},
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::Notify;

pub struct Source {
    pub reader: Box<dyn std::io::Read + Send>,
    pub kind: hermod_core::transfer::Kind,
    pub name: String,
    pub size: Option<u64>,
    pub cache: PathBuf,
    /// 真实文件路径（文件选择器来源）；有路径就能按 seek 发送（多流切片）。
    pub path: Option<PathBuf>,
}

pub struct DeviceContext {
    pub identity: Arc<hermod_core::identity::Identity>,
    pub contacts: hermod_core::contacts::Contacts,
    pub pair: bool,
}

pub enum Action {
    Send(Source),
    Receive {
        code: String,
        directory: PathBuf,
    },
    ReceiveDevice {
        peer: hermod_core::contacts::Pairing,
        directory: PathBuf,
    },
}

/// Core 内部稳定的传输阶段；GUI 通过枚举而不是英文文案判断状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferPhase {
    PreparingSource,
    ConnectingSignaling,
    WaitingForPeer,
    Sending,
    Receiving,
    Completed,
    Cancelled,
    Failed,
}

impl TransferPhase {
    /// 兼容旧 GUI 与通知的默认英文文案；新 GUI 应按枚举自行本地化。
    pub fn message(self) -> &'static str {
        match self {
            Self::PreparingSource => "Preparing document",
            Self::ConnectingSignaling => "Connecting to signaling server",
            Self::WaitingForPeer => "Waiting for the receiver",
            Self::Sending => "Sending file",
            Self::Receiving => "Receiving file",
            Self::Completed => "Transfer complete - SHA-256 verified",
            Self::Cancelled => "Transfer cancelled",
            Self::Failed => "Transfer failed",
        }
    }
}

#[derive(Clone)]
pub struct Snapshot {
    pub phase: TransferPhase,
    pub code: Option<String>,
    pub bytes: u64,
    pub total: Option<u64>,
    pub started: Instant,
    pub completed: Option<Instant>,
    pub path: Option<PathBuf>,
    pub digest: Option<String>,
    pub error: Option<String>,
    pub cancelled: bool,
    pub network: Option<NetworkStatsHandle>,
    pub transport: Option<String>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            phase: TransferPhase::ConnectingSignaling,
            code: None,
            bytes: 0,
            total: None,
            started: Instant::now(),
            completed: None,
            path: None,
            digest: None,
            error: None,
            cancelled: false,
            network: None,
            transport: None,
        }
    }
}

/// Every mutable access publishes before releasing the snapshot lock. Read access
/// uses `snapshot`, so observing state never manufactures an event.
struct PublishedState {
    snapshot: Mutex<Snapshot>,
    publish: Box<dyn Fn(Snapshot) + Send + Sync>,
}

struct PublishedGuard<'a> {
    guard: std::sync::MutexGuard<'a, Snapshot>,
    publish: &'a (dyn Fn(Snapshot) + Send + Sync),
}
impl std::ops::Deref for PublishedGuard<'_> {
    type Target = Snapshot;
    fn deref(&self) -> &Snapshot {
        &self.guard
    }
}
impl std::ops::DerefMut for PublishedGuard<'_> {
    fn deref_mut(&mut self) -> &mut Snapshot {
        &mut self.guard
    }
}
impl Drop for PublishedGuard<'_> {
    fn drop(&mut self) {
        (self.publish)(self.guard.clone());
    }
}
impl PublishedState {
    fn lock(&self) -> Result<PublishedGuard<'_>, String> {
        self.snapshot
            .lock()
            .map(|guard| PublishedGuard {
                guard,
                publish: &*self.publish,
            })
            .map_err(|_| "Snapshot lock poisoned".into())
    }
    fn snapshot(&self) -> Snapshot {
        self.snapshot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

pub struct Job {
    state: Arc<PublishedState>,
    cancel: Arc<Notify>,
    task: Option<tokio::task::JoinHandle<()>>,
    cancelled: Arc<std::sync::atomic::AtomicBool>,
    abort: Arc<Mutex<Option<AbortHandle>>>,
}

impl Job {
    pub fn start(
        config: Config,
        options: SessionOptions,
        action: Action,
        words: u8,
        on_finish: Box<dyn FnOnce() + Send>,
    ) -> Self {
        Self::start_with_device(config, options, action, words, on_finish, None)
    }
    pub fn start_with_device(
        config: Config,
        options: SessionOptions,
        action: Action,
        words: u8,
        on_finish: Box<dyn FnOnce() + Send>,
        device: Option<DeviceContext>,
    ) -> Self {
        Self::start_observed(
            config,
            options,
            action,
            words,
            on_finish,
            device,
            Box::new(|_| {}),
        )
    }
    pub(crate) fn start_observed(
        config: Config,
        options: SessionOptions,
        action: Action,
        words: u8,
        on_finish: Box<dyn FnOnce() + Send>,
        device: Option<DeviceContext>,
        publish: Box<dyn Fn(Snapshot) + Send + Sync>,
    ) -> Self {
        let initial = Snapshot::default();
        publish(initial.clone());
        let state = Arc::new(PublishedState {
            snapshot: Mutex::new(initial),
            publish,
        });
        let cancel = Arc::new(Notify::new());
        let abort = Arc::new(Mutex::new(None::<AbortHandle>));
        let mut job = Self {
            task: None,
            cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            state: state.clone(),
            cancel: cancel.clone(),
            abort: abort.clone(),
        };
        job.task = Some(tokio::spawn(async move {
            // Catch panics at the worker boundary so every accepted job reaches a terminal snapshot.
            use futures_util::FutureExt;
            let work = async {
                tokio::select! {
                    biased;
                    _ = cancel.notified() => {
                        if let Some(handle) = abort.lock().unwrap().take() { handle.abort(); }
                        let mut s = state.lock().unwrap();
                        if s.phase != TransferPhase::Completed {
                            s.cancelled = true;
                            s.phase = TransferPhase::Cancelled;
                        }
                        Ok(())
                    },
                    _ = idle_timeout(state.clone(), options.tuning.idle_timeout) => Err("Transfer made no progress before the idle timeout".into()),
                    result = run(config, options, action, words, state.clone(), abort.clone(), device) => result,
                }
            };
            let result = std::panic::AssertUnwindSafe(work)
                .catch_unwind()
                .await
                .unwrap_or_else(|_| Err("Transfer worker panicked".into()));
            if let Err(error) = result {
                if let Some(handle) = abort.lock().unwrap().take() {
                    handle.abort();
                }
                let mut s = state.lock().unwrap();
                s.phase = TransferPhase::Failed;
                s.error = Some(error);
            }
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(on_finish));
            state.lock().unwrap().completed = Some(Instant::now());
        }));
        job
    }

    pub fn snapshot(&self) -> Snapshot {
        self.state.snapshot()
    }
    pub fn cancel(&self) {
        let snapshot = self.snapshot();
        if snapshot.completed.is_none()
            && snapshot.phase != TransferPhase::Completed
            && !self
                .cancelled
                .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            if let Some(handle) = self.abort.lock().unwrap().take() {
                handle.abort();
            }
            self.cancel.notify_one();
        }
    }
    pub async fn shutdown(&mut self, deadline: Duration) -> Result<(), String> {
        self.cancel();
        if let Some(mut task) = self.task.take() {
            match tokio::time::timeout(deadline, &mut task).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => return Err(format!("Transfer task failed: {error}")),
                Err(_) => {
                    task.abort();
                    // Abort is cooperative; do not introduce an unbounded second join.
                    let _ = tokio::time::timeout(Duration::from_millis(250), &mut task).await;
                    let mut s = self.state.lock().unwrap();
                    if s.phase != TransferPhase::Completed {
                        s.cancelled = true;
                        s.phase = TransferPhase::Cancelled;
                    }
                    s.completed = Some(Instant::now());
                    return Err("Transfer shutdown timed out; task aborted".into());
                }
            }
        }
        Ok(())
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel();
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// 大文件没有总时长限制；只在数据和实际发包都停止时判定空闲超时。
async fn idle_timeout(state: Arc<PublishedState>, timeout: Duration) {
    let mut last = Instant::now();
    let mut bytes = 0;
    let mut wire = 0;
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let s = state.snapshot();
        if !matches!(s.phase, TransferPhase::Sending | TransferPhase::Receiving) {
            last = Instant::now();
            continue;
        }
        let sent = s
            .network
            .as_ref()
            .map(|n| n.snapshot().wire_bytes)
            .unwrap_or(0);
        if s.bytes != bytes || sent.saturating_sub(wire) >= 1200 {
            bytes = s.bytes;
            wire = sent;
            last = Instant::now();
        }
        if last.elapsed() >= timeout {
            return;
        }
    }
}

fn progress(state: Arc<PublishedState>) -> impl FnMut(u64) + Send {
    let mut last = Instant::now() - Duration::from_secs(1);
    move |bytes| {
        // GUI 只需 10 Hz，避免每个小限速块都锁界面状态、触发重绘。
        if last.elapsed() >= Duration::from_millis(100) {
            state.lock().unwrap().bytes = bytes;
            last = Instant::now();
        }
    }
}

async fn run(
    config: Config,
    mut options: SessionOptions,
    action: Action,
    words: u8,
    state: Arc<PublishedState>,
    abort: Arc<Mutex<Option<AbortHandle>>>,
    device: Option<DeviceContext>,
) -> Result<(), String> {
    match action {
        Action::Send(source) => {
            let mut temporary = Temporary(None);
            let size;
            let payload_source: orchestrate::PayloadSource;
            match (source.path.clone(), source.size) {
                (Some(path), Some(known)) => {
                    // 真实文件：路径直通（可 seek → 支持多流切片）。
                    size = known;
                    payload_source = orchestrate::PayloadSource::File(path);
                }
                (Some(path), None) => {
                    // 有路径但未知大小：直接取元数据，避免整份暂存。
                    let meta = tokio::fs::metadata(&path)
                        .await
                        .map_err(|e| e.to_string())?;
                    size = meta.len();
                    payload_source = orchestrate::PayloadSource::File(path);
                }
                (None, Some(known)) => {
                    // 内存/文本源：顺序流发送（单流模式）。
                    size = known;
                    payload_source =
                        orchestrate::PayloadSource::Stream(Box::new(stream_reader(source.reader)));
                }
                (None, None) => {
                    // 未知大小的文档流：暂存私有缓存；暂存文件本身可 seek。
                    state.lock().unwrap().phase = TransferPhase::PreparingSource;
                    let path = source.cache;
                    temporary.0 = Some(path.clone());
                    let mut file = Box::new(stream_reader(source.reader));
                    let mut staged = tokio::fs::OpenOptions::new()
                        .read(true)
                        .write(true)
                        .create_new(true)
                        .open(&path)
                        .await
                        .map_err(|e| e.to_string())?;
                    size = tokio::io::copy(&mut file, &mut staged)
                        .await
                        .map_err(|e| e.to_string())?;
                    payload_source = orchestrate::PayloadSource::File(path);
                }
            };
            options.offer = Some(hermod_core::transfer::TransferOffer {
                kind: source.kind,
                name: source.name.clone(),
                size,
            });
            // 可 seek 的文件源经 bundle 声明，接收端才可能选择多流切片模式。
            options.multi_stream_capable =
                matches!(&payload_source, orchestrate::PayloadSource::File(_));
            state.lock().unwrap().total = Some(size);
            let (_, code) = hermod_core::crypto::wordlist::generate_transfer_code(
                &mut rand_core::UnwrapErr(getrandom::SysRng),
                usize::from(words),
            );
            let _send_lease = if let Some(d) = &device {
                Some(d.contacts.acquire_send_lease().map_err(|e| e.to_string())?)
            } else {
                None
            };
            let mut tx = tokio::time::timeout(Duration::from_secs(660), async {
                let ready = || {
                    let mut s = state.lock().unwrap();
                    s.code = Some(code.clone());
                    s.phase = TransferPhase::WaitingForPeer;
                };
                if let Some(d) = &device {
                    session::sender_device_session(
                        &config,
                        &code,
                        &options,
                        &d.identity,
                        &d.contacts,
                        d.pair,
                        ready,
                    )
                    .await
                } else {
                    session::sender_session_with_ready(&config, &code, &options, ready).await
                }
            })
            .await
            .map_err(|_| "Timed out waiting for receiver".to_string())?
            .map_err(|e| e.to_string())?;
            *abort.lock().unwrap() = Some(tx.conn.abort_handle());
            {
                let mut s = state.lock().unwrap();
                s.phase = TransferPhase::Sending;
                s.started = Instant::now();
                s.network = tx.conn.network_stats();
                s.transport = Some(
                    match tx.conn {
                        hermod_core::transport::AnyConnection::Quic(_) => "quic",
                    }
                    .into(),
                );
            }
            let outcome = orchestrate::serve_file(
                &mut tx.conn,
                hermod_core::orchestrate::ServeOptions {
                    kind: source.kind,
                    name: source.name,
                    size,
                    source: payload_source,
                    progress: Some(Box::new(progress(state.clone()))),
                },
            )
            .await
            .map_err(|e| e.to_string())?;
            if !outcome.verified {
                return Err("Receiver digest verification failed".into());
            }
            tx.conn.shutdown().await.map_err(|e| e.to_string())?;
            abort.lock().unwrap().take();
            let mut s = state.lock().unwrap();
            s.bytes = size;
            s.digest = Some(outcome.sha256);
            s.phase = TransferPhase::Completed;
        }
        action @ (Action::Receive { .. } | Action::ReceiveDevice { .. }) => {
            let (code, peer, directory) = match action {
                Action::Receive { code, directory } => (Some(code), None, directory),
                Action::ReceiveDevice { peer, directory } => (None, Some(peer), directory),
                _ => unreachable!(),
            };
            let mut rx = tokio::time::timeout(Duration::from_secs(660), async {
                if let Some(peer) = &peer {
                    let d = device
                        .as_ref()
                        .ok_or_else(|| "Device context missing".to_string())?;
                    session::receiver_device_session(
                        &config,
                        peer,
                        &options,
                        &d.identity,
                        &d.contacts,
                    )
                    .await
                    .map_err(|e| e.to_string())
                } else {
                    let code = code.as_deref().ok_or("Receive code missing")?;
                    if let Some(d) = device.as_ref().filter(|d| d.pair) {
                        session::receiver_pairing_session(
                            &config,
                            code,
                            &options,
                            &d.identity,
                            &d.contacts,
                        )
                        .await
                        .map_err(|e| e.to_string())
                    } else {
                        session::receiver_session_with(&config, code, &options)
                            .await
                            .map_err(|e| e.to_string())
                    }
                }
            })
            .await
            .map_err(|_| "Peer handshake timed out".to_string())?
            .map_err(|e| e.to_string())?;
            *abort.lock().unwrap() = Some(rx.conn.abort_handle());
            {
                let mut s = state.lock().unwrap();
                s.total = rx.offer.as_ref().map(|offer| offer.size);
                s.phase = TransferPhase::Receiving;
                s.started = Instant::now();
                s.network = rx.conn.network_stats();
                s.transport = Some(
                    match rx.conn {
                        hermod_core::transport::AnyConnection::Quic(_) => "quic",
                    }
                    .into(),
                );
            }
            let mut on_progress = progress(state.clone());
            let receipt = orchestrate::receive(
                &mut rx.conn,
                &directory,
                orchestrate::ReceiveOptions {
                    offer: rx.offer.as_ref(),
                    progress: Some(&mut on_progress),
                    profile: options.profile,
                    peer_multi_stream: rx.peer_multi_stream,
                },
            )
            .await
            .map_err(|e| e.to_string())?;
            // No await between verified receive and durable receipt/terminal state.
            // Outer cancellation may stop cleanup, never revoke the verified inbox item.
            super::storage::record_verified(&directory, &receipt)?;
            {
                let mut s = state.lock().unwrap();
                s.bytes = receipt.size;
                s.total = Some(receipt.size);
                s.path = Some(receipt.path);
                s.digest = Some(receipt.sha256);
                s.phase = TransferPhase::Completed;
            }
            match tokio::time::timeout(Duration::from_secs(2), rx.conn.shutdown()).await {
                Ok(Ok(())) => {}
                _ => tracing::warn!("Verified inbox retained; connection cleanup did not complete"),
            }
            abort.lock().unwrap().take();
        }
    }
    Ok(())
}

/// 阻塞文档流在专用线程读取；1 MiB 块 × 16 深预读（与发送端流水线对齐，
/// 深度不足会把发送速率饿死在 ~128 KiB 在途），取消后关闭接收端。
fn stream_reader(
    mut reader: Box<dyn std::io::Read + Send>,
) -> impl tokio::io::AsyncRead + Unpin + Send {
    const CHUNK: usize = 1024 * 1024;
    const QUEUE: usize = 16;
    let (tx, rx) = tokio::sync::mpsc::channel::<std::io::Result<bytes::Bytes>>(QUEUE);
    tokio::task::spawn_blocking(move || {
        let failure = tx.clone();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            loop {
                if tx.is_closed() {
                    break;
                }
                let mut chunk = vec![0; CHUNK];
                match reader.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(count) => {
                        chunk.truncate(count);
                        if tx.blocking_send(Ok(bytes::Bytes::from(chunk))).is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        let _ = tx.blocking_send(Err(error));
                        break;
                    }
                }
            }
        }));
        if result.is_err() {
            let _ = failure.blocking_send(Err(std::io::Error::other("Source reader panicked")));
        }
    });
    let stream = futures_util::stream::unfold(rx, |mut rx| async {
        rx.recv().await.map(|item| (item, rx))
    });
    tokio_util::io::StreamReader::new(Box::pin(stream))
}

// 取消任务时也清理未知长度的文档缓存。
struct Temporary(Option<PathBuf>);
impl Drop for Temporary {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use hermod_core::signal::protocol::Message;
    use tokio_tungstenite::tungstenite::Message as Frame;

    fn source(path: &std::path::Path) -> Source {
        Source {
            reader: Box::new(std::fs::File::open(path).unwrap()),
            kind: hermod_core::transfer::Kind::File,
            name: "payload.bin".into(),
            size: None,
            cache: path.with_extension("staging"),
            path: None,
        }
    }

    #[tokio::test]
    async fn published_worker_cancel_is_terminal_and_observer_drop_does_not_cancel() {
        let hub = Arc::new(super::super::EventHub::new());
        let receiver = hub.subscribe().unwrap();
        assert_eq!(receiver.borrow().sequence, 0);
        assert!(receiver.borrow().status.is_none());
        let publish = hub.clone();
        let mut job = Job::start_observed(
            Config::default(),
            SessionOptions::default(),
            Action::Send(Source {
                reader: Box::new(std::io::Cursor::new(vec![1])),
                kind: hermod_core::transfer::Kind::File,
                name: "fixture.bin".into(),
                size: Some(1),
                cache: PathBuf::new(),
                path: None,
            }),
            3,
            Box::new(|| {}),
            None,
            Box::new(move |snapshot| publish.publish(snapshot)),
        );
        assert_eq!(receiver.borrow().sequence, 1);
        drop(receiver);
        assert!(!job.cancelled.load(std::sync::atomic::Ordering::Acquire));
        // Read-only observation must not publish synthetic changes.
        let mut receiver = hub.subscribe().unwrap();
        let sequence = receiver.borrow_and_update().sequence;
        let _ = job.snapshot();
        assert_eq!(receiver.borrow().sequence, sequence);
        job.shutdown(Duration::from_secs(1)).await.unwrap();
        let event = hub.subscribe().unwrap().borrow().clone();
        assert!(event.sequence > sequence);
        let status = event.status.unwrap();
        assert!(status.completed.is_some());
        assert!(status.cancelled);
        hub.close();
        // Consume the last real update before checking closure.
        receiver.borrow_and_update();
        assert!(receiver.changed().await.is_err());
    }

    #[tokio::test]
    async fn verified_completion_cannot_be_relabelled_by_cancel_or_shutdown_timeout() {
        let state = Arc::new(PublishedState {
            snapshot: Mutex::new(Snapshot {
                phase: TransferPhase::Completed,
                ..Default::default()
            }),
            publish: Box::new(|_| {}),
        });
        let mut job = Job {
            state,
            cancel: Arc::new(Notify::new()),
            task: Some(tokio::spawn(std::future::pending())),
            cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            abort: Arc::new(Mutex::new(None)),
        };
        job.cancel();
        assert!(!job.cancelled.load(std::sync::atomic::Ordering::Acquire));
        assert!(job.shutdown(Duration::from_millis(1)).await.is_err());
        assert_eq!(job.snapshot().phase, TransferPhase::Completed);
        assert!(!job.snapshot().cancelled);
    }

    #[tokio::test]
    async fn worker_panics_are_terminal_and_shutdown_is_bounded_and_idempotent() {
        struct PanicReader;
        impl std::io::Read for PanicReader {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                panic!("fixture")
            }
        }
        use tokio::io::AsyncReadExt;
        let mut reader = stream_reader(Box::new(PanicReader));
        assert!(
            reader
                .read_to_end(&mut Vec::new())
                .await
                .unwrap_err()
                .to_string()
                .contains("panicked")
        );
        let mut job = Job::start(
            Config::default(),
            SessionOptions::default(),
            Action::Send(Source {
                reader: Box::new(PanicReader),
                kind: hermod_core::transfer::Kind::File,
                name: "fixture.bin".into(),
                size: Some(1),
                cache: PathBuf::new(),
                path: None,
            }),
            3,
            Box::new(|| panic!("on_finish fixture")),
        );
        job.cancel();
        job.cancel();
        tokio::time::timeout(
            Duration::from_secs(1),
            job.shutdown(Duration::from_millis(100)),
        )
        .await
        .unwrap()
        .unwrap();
        job.shutdown(Duration::from_millis(100)).await.unwrap();
        let snapshot = job.snapshot();
        assert!(snapshot.completed.is_some());
        assert!(snapshot.cancelled);
        assert_eq!(snapshot.phase, TransferPhase::Cancelled);
    }

    /// 本地信令完成 CPace、文件校验、未知长度缓存和取消。
    #[test]
    fn desktop_jobs_transfer_a_file_and_cancel_while_waiting() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
          for (kind, pair) in [(hermod_core::transport::TransportKind::Quic, false), (hermod_core::transport::TransportKind::Quic, true)] {
            tokio::time::timeout(Duration::from_secs(15), async {
                let listener = tokio::net::TcpListener::bind("[::1]:0").await.unwrap();
                let mut cfg = Config { server_url: format!("ws://{}", listener.local_addr().unwrap()), ..Config::default() };
                cfg.trusted_servers.insert(cfg.server_url.clone(), "00".repeat(32));
                let relay = tokio::spawn(async move {
                    let (tcp, _) = listener.accept().await.unwrap();
                    let mut tx = tokio_tungstenite::accept_async(tcp).await.unwrap();
                    let allocate: Message = serde_json::from_str(tx.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
                    tx.send(Frame::text(serde_json::to_string(&Message::ok(allocate.channel_id, b"{}".to_vec())).unwrap())).await.unwrap();
                    let (tcp, _) = listener.accept().await.unwrap();
                    let mut rx = tokio_tungstenite::accept_async(tcp).await.unwrap();
                    let join: Message = serde_json::from_str(rx.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
                    assert_eq!(join.channel_id, allocate.channel_id);
                    rx.send(Frame::text(serde_json::to_string(&Message::ok(join.channel_id, b"{}".to_vec())).unwrap())).await.unwrap();
                    tx.send(Frame::text(serde_json::to_string(&Message::ready(join.channel_id)).unwrap())).await.unwrap();
                    loop {
                        tokio::select! {
                            Some(Ok(frame)) = tx.next() => { if rx.send(frame).await.is_err() { break; } },
                            Some(Ok(frame)) = rx.next() => { if tx.send(frame).await.is_err() { break; } },
                            else => break,
                        }
                    }
                });
                let dir = std::env::temp_dir().join(format!("hermod-desktop-test-{}", std::process::id()));
                let _ = std::fs::remove_dir_all(&dir);
                std::fs::create_dir_all(&dir).unwrap();
                let file = dir.join("payload.bin");
                let data = vec![0x53; 32_000];
                std::fs::write(&file, &data).unwrap();
                let options = SessionOptions { transport:Some(kind), udp_bind: None,
                    ..SessionOptions::default() };
                let device = |name: &str| {
                    let root = dir.join(name);
                    DeviceContext {
                        identity: Arc::new(hermod_core::identity::Identity::load_or_create(&root).unwrap()),
                        contacts: hermod_core::contacts::Contacts::open(&root).unwrap(), pair,
                    }
                };
                let sender_device = pair.then(|| device("sender"));
                let receiver_device = pair.then(|| device("receiver"));
                let sender_contacts = sender_device.as_ref().map(|d| d.contacts.clone());
                let receiver_contacts = receiver_device.as_ref().map(|d| d.contacts.clone());
                let tx = Job::start_with_device(cfg.clone(), options.clone(), Action::Send(source(&file)), 3, Box::new(|| {}), sender_device);
                let code = loop {
                    let s = tx.snapshot();
                    assert!(s.error.is_none(), "{:?}", s.error);
                    if let Some(code) = s.code { break code; }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                };
                let rx = Job::start_with_device(cfg, options.clone(), Action::Receive { code, directory: dir.join("out") }, 3, Box::new(|| {}), receiver_device);
                let mut announced_before_complete = false;
                loop {
                    let receiving = rx.snapshot();
                    if receiving.completed.is_none() && receiving.total == Some(data.len() as u64) { announced_before_complete = true; }
                    if tx.snapshot().completed.is_some() && rx.snapshot().completed.is_some() { break; }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                assert!(announced_before_complete, "receiver must know the size before completion");
                for job in [&tx, &rx] {
                    let s = job.snapshot();
                    assert!(s.error.is_none(), "{:?}", s.error);
                    assert_eq!(s.bytes, data.len() as u64);
                    assert_eq!(s.digest, Some(hermod_core::transfer::hash_bytes(&data)));
                }
                assert_eq!(std::fs::read(dir.join("out/payload.bin")).unwrap(), data);
                if pair {
                    for contacts in [sender_contacts.unwrap(), receiver_contacts.unwrap()] {
                        let peers = contacts.list().unwrap();
                        assert_eq!(peers.len(), 1);
                        assert_eq!(peers[0].state, hermod_core::contacts::PairingState::Active);
                    }
                }
                relay.await.unwrap();

                // 已收到 allocate 确认但尚无接收端时，取消也必须及时完成。
                let listener = tokio::net::TcpListener::bind("[::1]:0").await.unwrap();
                let mut cfg = Config { server_url: format!("ws://{}", listener.local_addr().unwrap()), ..Config::default() };
                cfg.trusted_servers.insert(cfg.server_url.clone(), "00".repeat(32));
                let relay = tokio::spawn(async move {
                    let (tcp, _) = listener.accept().await.unwrap();
                    let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
                    let request: Message = serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
                    ws.send(Frame::text(serde_json::to_string(&Message::ok(request.channel_id, b"{}".to_vec())).unwrap())).await.unwrap();
                    let _ = ws.next().await;
                });
                let pending = Job::start(cfg, options, Action::Send(source(&file)), 3, Box::new(|| {}));
                while pending.snapshot().code.is_none() {
                    assert!(pending.snapshot().error.is_none());
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                pending.cancel();
                while pending.snapshot().completed.is_none() { tokio::time::sleep(Duration::from_millis(10)).await; }
                assert!(pending.snapshot().cancelled);
                relay.await.unwrap();
                std::fs::remove_dir_all(dir).unwrap();
            }).await.expect("desktop transfer/cancellation timed out");
          }
        });
    }
}
