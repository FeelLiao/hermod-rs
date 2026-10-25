//! 单任务传输控制：进度、连接状态、取消和摘要，直接使用核心库而非解析 CLI 输出。

use hermod_core::transport::{AbortHandle, NetworkStatsHandle, TransportConnection};
use hermod_core::{
    bandwidth::BandwidthLimit,
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
}

pub enum Action {
    Send(Source),
    Receive { code: String, directory: PathBuf },
}

#[derive(Clone)]
pub struct Snapshot {
    pub phase: String,
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
            phase: "Connecting to signaling server".into(),
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

pub struct Job {
    pub state: Arc<Mutex<Snapshot>>,
    cancel: Arc<Notify>,
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
        let state = Arc::new(Mutex::new(Snapshot::default()));
        let cancel = Arc::new(Notify::new());
        let abort = Arc::new(Mutex::new(None::<AbortHandle>));
        let job = Self {
            state: state.clone(),
            cancel: cancel.clone(),
            abort: abort.clone(),
        };
        tokio::spawn(async move {
            let result = tokio::select! {
                biased;
                _ = cancel.notified() => {
                    if let Some(handle) = abort.lock().unwrap().take() { handle.abort(); }
                    let mut s = state.lock().unwrap();
                    s.cancelled = true;
                    s.phase = "Transfer cancelled".into();
                    Ok(())
                },
                _ = idle_timeout(state.clone()) => Err("Transfer made no progress for 90 seconds".into()),
                result = run(config, options, action, words, state.clone(), abort.clone()) => result,
            };
            if let Err(error) = result {
                if let Some(handle) = abort.lock().unwrap().take() {
                    handle.abort();
                }
                let mut s = state.lock().unwrap();
                s.phase = "Transfer failed".into();
                s.error = Some(error);
            }
            on_finish();
            state.lock().unwrap().completed = Some(Instant::now());
        });
        job
    }

    pub fn snapshot(&self) -> Snapshot {
        self.state.lock().unwrap().clone()
    }
    pub fn cancel(&self) {
        if self.snapshot().completed.is_none() {
            if let Some(handle) = self.abort.lock().unwrap().take() {
                handle.abort();
            }
            self.cancel.notify_one();
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// 大文件没有总时长限制；只在数据和实际发包都停止时判定空闲超时。
async fn idle_timeout(state: Arc<Mutex<Snapshot>>) {
    let mut last = Instant::now();
    let mut bytes = 0;
    let mut wire = 0;
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let s = state.lock().unwrap().clone();
        if s.phase != "Sending file" && s.phase != "Receiving file" {
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
        if last.elapsed() >= Duration::from_secs(90) {
            return;
        }
    }
}

fn progress(state: Arc<Mutex<Snapshot>>) -> impl FnMut(u64) + Send {
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
    options: SessionOptions,
    action: Action,
    words: u8,
    state: Arc<Mutex<Snapshot>>,
    abort: Arc<Mutex<Option<AbortHandle>>>,
) -> Result<(), String> {
    let limit: BandwidthLimit = options.bandwidth_limit;
    match action {
        Action::Send(source) => {
            let mut file: Box<dyn tokio::io::AsyncRead + Send + Unpin> =
                Box::new(stream_reader(source.reader));
            let mut temporary = Temporary(None);
            let size = if let Some(size) = source.size {
                size
            } else {
                state.lock().unwrap().phase = "Preparing document".into();
                let path = source.cache;
                temporary.0 = Some(path.clone());
                let mut staged = tokio::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .await
                    .map_err(|e| e.to_string())?;
                let size = tokio::io::copy(&mut file, &mut staged)
                    .await
                    .map_err(|e| e.to_string())?;
                use tokio::io::AsyncSeekExt;
                staged.rewind().await.map_err(|e| e.to_string())?;
                file = Box::new(staged);
                size
            };
            state.lock().unwrap().total = Some(size);
            let (_, code) = hermod_core::crypto::wordlist::generate_transfer_code(
                &mut rand_core::UnwrapErr(getrandom::SysRng),
                usize::from(words),
            );
            let mut tx = tokio::time::timeout(
                Duration::from_secs(660),
                session::sender_session_with_ready(&config, &code, &options, || {
                    let mut s = state.lock().unwrap();
                    s.code = Some(code.clone());
                    s.phase = "Waiting for the receiver".into();
                }),
            )
            .await
            .map_err(|_| "Timed out waiting for receiver".to_string())?
            .map_err(|e| e.to_string())?;
            *abort.lock().unwrap() = Some(tx.conn.abort_handle());
            {
                let mut s = state.lock().unwrap();
                s.phase = "Sending file".into();
                s.started = Instant::now();
                s.network = tx.conn.network_stats();
                s.transport = Some(
                    match tx.conn {
                        hermod_core::transport::AnyConnection::WireGuard(_) => "wireguard",
                        hermod_core::transport::AnyConnection::Quic(_) => "quic",
                    }
                    .into(),
                );
            }
            let outcome = orchestrate::serve_file_with_limit(
                &mut tx.conn,
                hermod_core::transport::http::server::ServeOptions {
                    token: tx.token,
                    kind: source.kind,
                    name: source.name,
                    size,
                    body: file,
                    progress: Some(Box::new(progress(state.clone()))),
                },
                limit,
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
            s.phase = "Transfer complete - SHA-256 verified".into();
        }
        Action::Receive { code, directory } => {
            let mut rx = tokio::time::timeout(
                Duration::from_secs(660),
                session::receiver_session_with(&config, &code, &options),
            )
            .await
            .map_err(|_| "Peer handshake timed out".to_string())?
            .map_err(|e| e.to_string())?;
            *abort.lock().unwrap() = Some(rx.conn.abort_handle());
            {
                let mut s = state.lock().unwrap();
                s.phase = "Receiving file".into();
                s.started = Instant::now();
                s.network = rx.conn.network_stats();
                s.transport = Some(
                    match rx.conn {
                        hermod_core::transport::AnyConnection::WireGuard(_) => "wireguard",
                        hermod_core::transport::AnyConnection::Quic(_) => "quic",
                    }
                    .into(),
                );
            }
            let mut on_progress = progress(state.clone());
            let receipt = orchestrate::receive_with_limit(
                &mut rx.conn,
                &rx.token,
                &directory,
                Some(&mut on_progress),
                limit,
            )
            .await
            .map_err(|e| e.to_string())?;
            rx.conn.shutdown().await.map_err(|e| e.to_string())?;
            abort.lock().unwrap().take();
            super::storage::record(&directory, &receipt).await?;
            let mut s = state.lock().unwrap();
            s.bytes = receipt.size;
            s.total = Some(receipt.size);
            s.path = Some(receipt.path);
            s.digest = Some(receipt.sha256);
            s.phase = "Transfer complete - SHA-256 verified".into();
        }
    }
    Ok(())
}

/// 阻塞文档流在专用线程读取；最多缓存两个 64 KiB 块，取消后关闭接收端。
fn stream_reader(
    mut reader: Box<dyn std::io::Read + Send>,
) -> impl tokio::io::AsyncRead + Unpin + Send {
    let (tx, rx) = tokio::sync::mpsc::channel::<std::io::Result<bytes::Bytes>>(2);
    tokio::task::spawn_blocking(move || {
        loop {
            if tx.is_closed() {
                break;
            }
            let mut chunk = vec![0; 64 * 1024];
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
        }
    }

    /// 两种协议经本地信令完成 CPace、文件校验、未知长度缓存和取消。
    #[test]
    fn desktop_jobs_transfer_a_file_and_cancel_while_waiting() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
          for kind in [hermod_core::transport::TransportKind::WireGuard, hermod_core::transport::TransportKind::Quic] {
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
                    bandwidth_limit: "0.8".parse().unwrap(), ..SessionOptions::default() };
                let tx = Job::start(cfg.clone(), options.clone(), Action::Send(source(&file)), 3, Box::new(|| {}));
                let code = loop {
                    let s = tx.snapshot();
                    assert!(s.error.is_none(), "{:?}", s.error);
                    if let Some(code) = s.code { break code; }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                };
                let rx = Job::start(cfg, options.clone(), Action::Receive { code, directory: dir.join("out") }, 3, Box::new(|| {}));
                loop {
                    if tx.snapshot().completed.is_some() && rx.snapshot().completed.is_some() { break; }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                for job in [&tx, &rx] {
                    let s = job.snapshot();
                    assert!(s.error.is_none(), "{:?}", s.error);
                    assert_eq!(s.bytes, data.len() as u64);
                    assert_eq!(s.digest, Some(hermod_core::transfer::hash_bytes(&data)));
                }
                assert_eq!(std::fs::read(dir.join("out/payload.bin")).unwrap(), data);
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
