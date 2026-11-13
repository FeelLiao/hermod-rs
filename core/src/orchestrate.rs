//! 传输编排（协议 v3）：把「传输连接」与「直连流帧」组合成 tx / rx 两个动作。
//!
//! ## 数据面模式（v3 起由接收端决定）
//!
//! - **单流**（兼容/弱网档，或小文件）：一条双向流跑 v2 形状——
//!   `MODE_SINGLE` → 精确写 `size` 字节载荷 → 裁决帧；
//! - **多流**（局域网档大文件）：控制流先发 `MODE_MULTI` + 切片计划
//!   `{slice_size: u64, slice_count: u32}`，随后按切片逐条开流（每条带
//!   32 字节会话前导 + `slice_index: u32`），发送端按索引 seek 源文件发送
//!   切片；全部切片完成后控制流回裁决帧。接收端按偏移写盘 + 顺序读回切片
//!   计算 SHA-256，不持有乱序重组内存，磁盘读回走页缓存。
//!
//! 多流的意义（WiFi 抗丢包）：单流上一次丢包恢复会阻塞整条流的应用层交付，
//! 多流把阻塞面限制在单个切片内，其余流继续灌满链路。
//!
//! 能力协商：发送端源是否可 seek 在端点交换时经 bundle 的 `multi_stream`
//! 字段公布（[`crate::session`] 搬运）；接收端只在该标志为真、自身档位选择
//! 多流且文件足够大时才用多流模式。
//!
//! 对端可控的 offer 早经 [`crate::transfer::TransferOffer::validate`]（含文件名
//! 卫生）校验；对端取消统一映射为 [`OrchestrateError::PeerCancelled`]。

use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::sync::{Notify, mpsc};

use crate::options::TransferTuning as ConnectionTuning;
use crate::transfer::TransferOffer;
use futures_util::stream::{FuturesUnordered, StreamExt};
use sha2::{Digest, Sha256};

/// 数据面模式：单流（v2 形状）。
const MODE_SINGLE: u8 = 0x01;
/// 数据面模式：多流切片。
const MODE_MULTI: u8 = 0x02;
/// 裁决状态：核算通过。
const VERDICT_OK: u8 = 0;
/// 裁决帧字节长度：status(1) + sha256(32) + size(u64 LE)。
const VERDICT_LEN: usize = 1 + 32 + 8;
/// 切片计划帧字节长度：slice_size(u64 LE) + slice_count(u32 LE)。
const PLAN_LEN: usize = 12;
/// 计划数量上限保持 v3 兼容；与实际同时运行的数据流数量无关。
const MAX_SLICES: u64 = 512;
/// 多流模式的最小文件尺寸：小文件用多流是纯开销。
const MIN_MULTISTREAM_SIZE: u64 = 16 * 1024 * 1024;
/// 切片目标数量：文件再大也按 256 片划分（切片随尺寸自动放大）。
const TARGET_SLICES: u64 = 256;
/// 数据流并行度上限；控制流独立占用一条流，不受本端并发偏好限制。
const MAX_STREAMS: usize = 8;
/// Auto 档的局域网 RTT 判定阈值；不高于它按局域网档并发。
const LAN_RTT_THRESHOLD: Duration = Duration::from_millis(25);

/// 编排错误：传输层、IO、元数据或裁决。
#[derive(Debug, thiserror::Error)]
pub enum OrchestrateError {
    #[error("transport: {0}")]
    Transport(String),
    #[error("io: {0}")]
    Io(io::Error),
    #[error("transfer: {0}")]
    Transfer(#[from] crate::transfer::TransferError),
    #[error("digest mismatch: rx 提交的核算结果未被 tx 接受")]
    DigestMismatch,
    /// 对端中止了连接（Ctrl+C / CONNECTION_CLOSE / 提前 EOF）。
    #[error("peer cancelled the transfer")]
    PeerCancelled,
    #[error("protocol violation: {0}")]
    Protocol(String),
}

/// 把 IO 错误归类：显式取消标记或常见断连都按"对端取消"呈现。
fn classify_io(error: io::Error) -> OrchestrateError {
    if crate::transport::iface::is_peer_cancelled(&error) {
        return OrchestrateError::PeerCancelled;
    }
    match error.kind() {
        io::ErrorKind::BrokenPipe
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::ConnectionAborted
        | io::ErrorKind::UnexpectedEof => OrchestrateError::PeerCancelled,
        _ => OrchestrateError::Io(error),
    }
}

fn hex(bytes: &[u8]) -> String {
    crate::transfer::hex(bytes)
}

/// rx 成功落盘后的回执。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    /// 最终落盘路径（已去重）。
    pub path: std::path::PathBuf,
    /// SHA-256（十六进制小写）。
    pub sha256: String,
    /// 字节数。
    pub size: u64,
    /// 载荷类型。
    pub kind: crate::transfer::Kind,
}

/// tx 侧载荷来源。
///
/// 文件路径支持按切片 seek（多流模式的前提）；顺序流（stdin/内存/文档流）
/// 只能走单流模式，能力标志由上层在端点交换时如实公布。
pub enum PayloadSource {
    /// 可随机访问的本地文件。
    File(std::path::PathBuf),
    /// 顺序流（边发边读，不整体进内存）。
    Stream(Box<dyn AsyncRead + Unpin + Send>),
}

/// tx 侧载荷元数据与来源（size/name/kind 必须真实，且已经
/// [`crate::transfer::TransferOffer::validate`] 校验过——session 层在端点交换时做的）。
pub struct ServeOptions {
    /// 载荷类型（回执与目录展示用）。
    pub kind: crate::transfer::Kind,
    /// 原始文件名（对端落盘时使用）。
    pub name: String,
    /// 将写入载荷的精确字节数（与 offer 一致）。
    pub size: u64,
    /// 载荷来源（文件路径 / 顺序流）。
    pub source: PayloadSource,
    /// 进度回调：参数是**累计已发字节数**。
    pub progress: Option<Box<dyn FnMut(u64) + Send>>,
}

/// 服务一条连接的结果：rx 裁决是否与本地一致。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServeOutcome {
    /// rx 提交的 SHA-256/大小是否与 tx 本地一致。
    pub verified: bool,
    /// tx 边发边算出的 SHA-256（十六进制小写）。
    pub sha256: String,
}

/// rx 侧接收参数。
#[derive(Default)]
pub struct ReceiveOptions<'a> {
    /// 端点交换阶段宣告过的载荷（提供 size/name/kind；v2 下必填）。
    pub offer: Option<&'a TransferOffer>,
    /// 进度回调：参数是**累计已收字节数**。
    pub progress: Option<&'a mut (dyn FnMut(u64) + Send)>,
    /// 本端链路档位（Auto 按 RTT 在线决定并发）。
    pub profile: crate::options::LinkProfile,
    /// 对端（发送方）公布的载荷可 seek 能力（bundle `multi_stream`）。
    pub peer_multi_stream: bool,
}

/// 解析本次接收的并发预算；显式覆盖仍受能力与尺寸闸门约束。
fn resolve_streams(
    tuning: ConnectionTuning,
    profile: crate::options::LinkProfile,
    peer: Option<std::net::SocketAddr>,
    rtt: Option<Duration>,
    size: u64,
    peer_multi_stream: bool,
) -> usize {
    if !peer_multi_stream || size < MIN_MULTISTREAM_SIZE {
        return 1;
    }
    if let Some(streams) = tuning.streams {
        return streams;
    }
    let lan = match profile {
        crate::options::LinkProfile::Lan => true,
        crate::options::LinkProfile::Wan => false,
        crate::options::LinkProfile::Auto => {
            peer.is_some_and(|addr| {
                crate::transport::network::classify_link(addr)
                    == crate::transport::network::LinkKind::Lan
            }) && rtt
                .filter(|rtt| !rtt.is_zero())
                .is_none_or(|rtt| rtt <= LAN_RTT_THRESHOLD)
        }
    };
    if lan { MAX_STREAMS } else { 1 }
}

/// 由文件尺寸推导切片计划；切片尺寸独立于本地搬运块大小。
fn slice_plan(size: u64, slice_target: u64) -> (u64, u64) {
    debug_assert!(size >= MIN_MULTISTREAM_SIZE);
    let target = slice_target.max(1).min(size.div_ceil(2));
    let count = size.div_ceil(target).clamp(2, TARGET_SLICES);
    let slice_size = size.div_ceil(count);
    (slice_size, size.div_ceil(slice_size))
}

/// 校验对端发来的切片计划（对端可控；越界即协议违约）。
fn validate_plan(size: u64, slice_size: u64, slice_count: u64) -> Result<(), OrchestrateError> {
    // 除法等价于精确覆盖检查，避免对端控制的乘法溢出；不依赖本端 chunk。
    let ok = slice_size > 0
        && (2..=MAX_SLICES).contains(&slice_count)
        && size.div_ceil(slice_size) == slice_count;
    if ok {
        Ok(())
    } else {
        Err(OrchestrateError::Protocol(format!(
            "bad slice plan: size={size} slice_size={slice_size} slice_count={slice_count}"
        )))
    }
}

/// 有界预读：把 `AsyncRead` 的读取搬进后台任务，经本端预算指定容量的
/// 队列递送，让磁盘读取与下游处理（哈希 / 写流）重叠——单流路径不再每块
/// 串行等磁盘。
struct PrefetchReader {
    rx: mpsc::Receiver<io::Result<Vec<u8>>>,
    current: Vec<u8>,
    pos: usize,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for PrefetchReader {
    fn drop(&mut self) {
        // 丢弃上游时连带取消阻塞中的预读，不能只依赖队列接收端关闭。
        self.task.abort();
    }
}

impl PrefetchReader {
    async fn stop(&mut self) {
        self.task.abort();
        let _ = (&mut self.task).await;
    }
}

impl AsyncRead for PrefetchReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        loop {
            if self.pos < self.current.len() {
                let n = (self.current.len() - self.pos).min(buf.remaining());
                buf.put_slice(&self.current[self.pos..self.pos + n]);
                self.pos += n;
                return Poll::Ready(Ok(()));
            }
            match std::task::ready!(Pin::new(&mut self.rx).poll_recv(cx)) {
                Some(Ok(chunk)) => {
                    self.current = chunk;
                    self.pos = 0;
                }
                Some(Err(error)) => return Poll::Ready(Err(error)),
                None => return Poll::Ready(Ok(())), // EOF
            }
        }
    }
}

/// 把 `reader` 搬进预读任务，返回可读的适配器。
fn prefetch<R>(reader: R, tuning: ConnectionTuning) -> PrefetchReader
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let (tx, rx) = mpsc::channel(tuning.prefetch.max(1));
    let task = tokio::spawn(async move {
        let mut reader = reader;
        loop {
            let mut buf = vec![0u8; tuning.chunk.max(1)];
            match reader.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    buf.truncate(n);
                    if tx.send(Ok(buf)).await.is_err() {
                        break;
                    }
                }
                Err(error) => {
                    let _ = tx.send(Err(error)).await;
                    break;
                }
            }
        }
    });
    PrefetchReader {
        rx,
        current: Vec::new(),
        pos: 0,
        task,
    }
}

/// 传输统计摘要：结束事件带阶段耗时与网络层计数（日志级观测，RUST_LOG 控制）。
struct Summary {
    role: &'static str,
    start: Instant,
    stats: Option<crate::transport::NetworkStatsHandle>,
    before: Option<crate::transport::NetworkStats>,
}

impl Summary {
    fn new<C: crate::transport::TransportConnection>(role: &'static str, conn: &C) -> Self {
        let stats = conn.network_stats();
        let before = stats.as_ref().map(|h| h.snapshot());
        Self {
            role,
            start: Instant::now(),
            stats,
            before,
        }
    }

    /// 结束事件：模式、并发、耗时与网络层计数增量。
    fn finish(&self, mode: &str, streams: usize, bytes: u64) {
        let secs = self.start.elapsed().as_secs_f64();
        let mbps = bytes as f64 * 8.0 / 1e6 / secs.max(1e-9);
        match (self.stats.as_ref(), self.before) {
            (Some(handle), Some(before)) => {
                let after = handle.snapshot();
                crate::log_info!(
                    role = self.role,
                    mode,
                    streams,
                    bytes,
                    secs,
                    mbps,
                    wire_bytes = after.wire_bytes.saturating_sub(before.wire_bytes),
                    lost_packets = after.lost_packets.saturating_sub(before.lost_packets),
                    lost_bytes = after.lost_bytes.saturating_sub(before.lost_bytes),
                    cwnd = after.cwnd,
                    rtt_ms = after.rtt.as_millis() as u64,
                    "transfer summary"
                );
            }
            _ => crate::log_info!(
                role = self.role,
                mode,
                streams,
                bytes,
                secs,
                mbps,
                "transfer summary"
            ),
        }
    }
}
/// tx：在传输内监听、按对端选择的数据面模式发完载荷并读完裁决。
///
/// 对应 [`send_file`]/[`send_text`] 的组合体；供自定义载荷来源使用。
pub async fn serve_file<C>(
    conn: &mut C,
    opts: ServeOptions,
) -> Result<ServeOutcome, OrchestrateError>
where
    C: crate::transport::TransportConnection,
{
    if let Err(error) = crate::transfer::validate_filename(&opts.name) {
        return Err(OrchestrateError::Protocol(error.to_string()));
    }
    let tuning = conn.transfer_tuning();
    tuning.validate().map_err(OrchestrateError::Protocol)?;
    let summary = Summary::new("tx", conn);
    let mut stream = conn
        .listen()
        .await
        .map_err(|e| OrchestrateError::Transport(e.to_string()))?;
    let mut first = [0u8; 1];
    stream.read_exact(&mut first).await.map_err(classify_io)?;
    match first[0] {
        MODE_SINGLE => serve_single(summary, stream, opts, tuning).await,
        MODE_MULTI => serve_multi(conn, summary, stream, opts).await,
        other => Err(OrchestrateError::Protocol(format!(
            "unexpected data-plane mode {other:#04x}"
        ))),
    }
}

/// tx：单流模式（v2 形状）。载荷来源经预读流水线，读盘与写流重叠。
/// 模式字节已在 [`serve_file`] 读取——它同时是 v2 语义里的 GO（对端就绪）。
async fn serve_single<S>(
    summary: Summary,
    mut stream: S,
    mut opts: ServeOptions,
    tuning: ConnectionTuning,
) -> Result<ServeOutcome, OrchestrateError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    // 1) 精确写 size 字节载荷，边发边算 SHA-256（来源经预读）
    let source = std::mem::replace(
        &mut opts.source,
        PayloadSource::Stream(Box::new(tokio::io::empty())),
    );
    let mut body = match source {
        PayloadSource::Stream(reader) => prefetch(reader, tuning),
        PayloadSource::File(path) => {
            let file = tokio::fs::File::open(&path).await.map_err(classify_io)?;
            prefetch(file, tuning)
        }
    };
    let mut hasher = Sha256::new();
    let mut written: u64 = 0;
    let mut remaining = opts.size;
    while remaining > 0 {
        let want = (tuning.chunk.max(1) as u64).min(remaining) as usize;
        let mut buf = vec![0u8; want];
        let n = body.read(&mut buf).await.map_err(classify_io)?;
        if n == 0 {
            return Err(OrchestrateError::Protocol(
                "payload shorter than announced size".into(),
            ));
        }
        hasher.update(&buf[..n]);
        stream.write_all(&buf[..n]).await.map_err(classify_io)?;
        written += n as u64;
        remaining -= n as u64;
        if let Some(progress) = opts.progress.as_mut() {
            progress(written);
        }
    }
    body.stop().await;
    stream.flush().await.map_err(classify_io)?;
    let mut local_sha_bytes = [0u8; 32];
    local_sha_bytes.copy_from_slice(&hasher.finalize());
    let local_sha = hex(&local_sha_bytes);

    // 3) 读裁决：status(1) + sha256(32) + size(8)
    let verified = read_verdict(&mut stream, &local_sha_bytes, opts.size).await?;
    summary.finish("single", 1, written);
    Ok(ServeOutcome {
        verified,
        sha256: local_sha,
    })
}

/// 等待新流时继续推进已有切片，且不取消带认证前导的建流 future。
async fn await_stream<F, T, E, W>(
    opening: F,
    tasks: &mut FuturesUnordered<W>,
    tick: &mut tokio::time::Interval,
    mut report: impl FnMut(),
) -> Result<T, OrchestrateError>
where
    F: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Display,
    W: std::future::Future<Output = Result<(), OrchestrateError>>,
{
    tokio::pin!(opening);
    loop {
        tokio::select! {
            stream = &mut opening => return stream.map_err(|e| OrchestrateError::Transport(e.to_string())),
            _ = tick.tick() => report(),
            item = tasks.next(), if !tasks.is_empty() => {
                if let Some(result) = item { result?; }
            }
        }
    }
}

/// tx：多流切片模式。控制流读计划，按对端开来的切片流 seek 源文件发送。
async fn serve_multi<C>(
    conn: &mut C,
    summary: Summary,
    mut control: C::Stream,
    mut opts: ServeOptions,
) -> Result<ServeOutcome, OrchestrateError>
where
    C: crate::transport::TransportConnection,
{
    let tuning = conn.transfer_tuning();
    // 1) 切片计划（对端=接收端决定）
    let mut plan = [0u8; PLAN_LEN];
    control.read_exact(&mut plan).await.map_err(classify_io)?;
    let slice_size = u64::from_le_bytes(plan[0..8].try_into().unwrap());
    let slice_count = u64::from(u32::from_le_bytes(plan[8..12].try_into().unwrap()));
    validate_plan(opts.size, slice_size, slice_count)?;
    // 2) 源必须可 seek（对端能力标志保证了这一点；防御性再查一次）
    let path = match std::mem::replace(
        &mut opts.source,
        PayloadSource::Stream(Box::new(tokio::io::empty())),
    ) {
        PayloadSource::File(path) => path,
        PayloadSource::Stream(_) => {
            return Err(OrchestrateError::Protocol("source is not seekable".into()));
        }
    };

    // 3) 本地哈希：独立任务顺序读源文件（与发送并行；页缓存让二次读很便宜）
    let hash_task = hash_file(path.clone(), opts.size, tuning);

    // 4) 逐个接受切片流，按对端的索引 seek 发送对应切片
    let sent = Arc::new(AtomicU64::new(0));
    let mut tasks = FuturesUnordered::new();
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut accepted = 0;
    let mut seen = vec![false; slice_count as usize];
    let sending = async {
        while accepted < slice_count || !tasks.is_empty() {
            if accepted < slice_count && tasks.len() < MAX_STREAMS {
                let mut stream = await_stream(conn.listen(), &mut tasks, &mut tick, || {
                    if let Some(progress) = opts.progress.as_mut() {
                        progress(sent.load(Ordering::Relaxed));
                    }
                })
                .await?;
                let mut index = [0u8; 4];
                stream.read_exact(&mut index).await.map_err(classify_io)?;
                let index = u32::from_le_bytes(index) as u64;
                if index >= slice_count {
                    return Err(OrchestrateError::Protocol(format!(
                        "slice index {index} out of range {slice_count}"
                    )));
                }
                if std::mem::replace(&mut seen[index as usize], true) {
                    return Err(OrchestrateError::Protocol(format!(
                        "duplicate slice index {index}"
                    )));
                }
                accepted += 1;
                let offset = index * slice_size;
                let len = (opts.size - offset).min(slice_size);
                let (path2, sent2) = (path.clone(), sent.clone());
                tasks.push(
                    async move { serve_slice(path2, stream, offset, len, sent2, tuning).await },
                );
                continue;
            }
            tokio::select! {
                result = tasks.next(), if !tasks.is_empty() => {
                    if let Some(result) = result { result?; }
                }
                _ = tick.tick() => {
                    if let Some(progress) = opts.progress.as_mut() {
                        progress(sent.load(Ordering::Relaxed));
                    }
                }
            }
        }
        Ok::<(), OrchestrateError>(())
    };
    // 5) 本地核算与发送并行；任一失败会释放另一侧 future。
    let (_, local_sha_bytes) = tokio::try_join!(sending, hash_task)?;
    let local_sha = hex(&local_sha_bytes);
    let verified = read_verdict(&mut control, &local_sha_bytes, opts.size).await?;
    summary.finish("multi", slice_count as usize, opts.size);
    Ok(ServeOutcome {
        verified,
        sha256: local_sha,
    })
}

/// tx：发送一个切片——seek 到偏移，精确写 len 字节（进度进共享计数）。
async fn serve_slice<S>(
    path: std::path::PathBuf,
    mut stream: S,
    offset: u64,
    len: u64,
    sent: Arc<AtomicU64>,
    tuning: ConnectionTuning,
) -> Result<(), OrchestrateError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let mut file = tokio::fs::File::open(&path).await.map_err(classify_io)?;
    file.seek(io::SeekFrom::Start(offset))
        .await
        .map_err(classify_io)?;
    let mut remaining = len;
    let mut buf = vec![0u8; tuning.chunk.max(1)];
    while remaining > 0 {
        let want = (tuning.chunk.max(1) as u64).min(remaining) as usize;
        let n = file.read(&mut buf[..want]).await.map_err(classify_io)?;
        if n == 0 {
            return Err(OrchestrateError::Protocol(
                "payload shorter than file".into(),
            ));
        }
        stream.write_all(&buf[..n]).await.map_err(classify_io)?;
        sent.fetch_add(n as u64, Ordering::Relaxed);
        remaining -= n as u64;
    }
    stream.flush().await.map_err(classify_io)?;
    // finish + 等 stopped：确保对端收满本切片（取消时这里映射为 PeerCancelled）。
    stream.shutdown().await.map_err(classify_io)
}

/// 顺序读整个文件并计算 SHA-256（tx 多流模式下的本地核算，与发送并行）。
async fn hash_file(
    path: std::path::PathBuf,
    size: u64,
    tuning: ConnectionTuning,
) -> Result<[u8; 32], OrchestrateError> {
    let mut file = tokio::fs::File::open(&path).await.map_err(classify_io)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; tuning.chunk.max(1)];
    let mut remaining = size;
    while remaining > 0 {
        let want = (tuning.chunk.max(1) as u64).min(remaining) as usize;
        let n = file.read(&mut buf[..want]).await.map_err(classify_io)?;
        if n == 0 {
            return Err(OrchestrateError::Protocol(
                "file shorter than announced".into(),
            ));
        }
        hasher.update(&buf[..n]);
        remaining -= n as u64;
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&hasher.finalize());
    Ok(out)
}

/// 读裁决帧并核对 status/sha/size。
async fn read_verdict<S>(
    stream: &mut S,
    local_sha_bytes: &[u8; 32],
    size: u64,
) -> Result<bool, OrchestrateError>
where
    S: AsyncRead + Unpin,
{
    let mut verdict = [0u8; VERDICT_LEN];
    stream.read_exact(&mut verdict).await.map_err(classify_io)?;
    Ok(verdict[0] == VERDICT_OK
        && verdict[1..33] == local_sha_bytes[..]
        && u64::from_le_bytes(verdict[33..41].try_into().unwrap()) == size)
}

/// 切片完成跟踪：每个切片一个标志 + 一个通知；哈希任务按序等待。
/// `notify_one` 的存储许可语义保证「先置位后等待」不丢唤醒。
struct SliceDone {
    flags: Vec<AtomicBool>,
    notify: Notify,
}

impl SliceDone {
    fn new(count: u64) -> Self {
        Self {
            flags: (0..count).map(|_| AtomicBool::new(false)).collect(),
            notify: Notify::new(),
        }
    }

    fn mark(&self, index: usize) {
        self.flags[index].store(true, Ordering::Release);
        self.notify.notify_one();
    }

    /// 等待第 `index` 个切片完成写入（哈希按序消费）。
    async fn wait_for(&self, index: usize) {
        while !self.flags[index].load(Ordering::Acquire) {
            self.notify.notified().await;
        }
    }
}

/// rx：单流模式（v2 形状）。读满 offer.size 字节，边读边写、边算 SHA-256。
/// 读半边进预读流水线；写半边留给裁决帧。
async fn fetch_single<S>(
    stream: S,
    offer: &TransferOffer,
    temp: &std::path::Path,
    mut progress: Option<&mut (dyn FnMut(u64) + Send)>,
    tuning: ConnectionTuning,
) -> Result<(String, bool), OrchestrateError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let mut control = stream;
    // 1) GO：同意接收（触发 tx 的 accept_bi）
    control
        .write_all(&[MODE_SINGLE])
        .await
        .map_err(classify_io)?;
    let mut dest = open_temp_rw(temp).await?;
    let (read_half, mut write_half) = tokio::io::split(control);
    let mut body = prefetch(read_half.take(offer.size), tuning);

    // 2) 精确读 offer.size 字节，边读边写、边算 SHA-256
    let mut hasher = Sha256::new();
    let mut received: u64 = 0;
    let mut remaining = offer.size;
    while remaining > 0 {
        let want = (tuning.chunk.max(1) as u64).min(remaining) as usize;
        let mut buf = vec![0u8; want];
        let n = body.read(&mut buf).await.map_err(classify_io)?;
        if n == 0 {
            return Err(OrchestrateError::Protocol(
                "payload shorter than announced size".into(),
            ));
        }
        dest.write_all(&buf[..n]).await.map_err(classify_io)?;
        hasher.update(&buf[..n]);
        received += n as u64;
        remaining -= n as u64;
        if let Some(progress) = progress.as_mut() {
            progress(received);
        }
    }
    body.stop().await;
    dest.flush().await.map_err(classify_io)?;
    drop(dest);
    let mut local_sha_bytes = [0u8; 32];
    local_sha_bytes.copy_from_slice(&hasher.finalize());

    // 3) 裁决帧：status + sha256 + size（诚实 rx：按本地核算提交）
    let mut verdict = [0u8; VERDICT_LEN];
    verdict[0] = VERDICT_OK;
    verdict[1..33].copy_from_slice(&local_sha_bytes);
    verdict[33..41].copy_from_slice(&received.to_le_bytes());
    write_half.write_all(&verdict).await.map_err(classify_io)?;
    write_half.shutdown().await.map_err(classify_io)?;
    Ok((hex(&local_sha_bytes), received == offer.size))
}

/// rx：多流切片模式。控制流发计划，按切片开流收字节、按偏移写盘，
/// 哈希任务按序读回切片核算（不持有乱序重组内存）。
async fn fetch_multi<C>(
    conn: &mut C,
    mut control: C::Stream,
    offer: &TransferOffer,
    temp: &std::path::Path,
    mut progress: Option<&mut (dyn FnMut(u64) + Send)>,
    tuning: ConnectionTuning,
) -> Result<(String, bool), OrchestrateError>
where
    C: crate::transport::TransportConnection,
{
    let streams = tuning.streams.unwrap_or(MAX_STREAMS);
    let (slice_size, slice_count) = slice_plan(offer.size, tuning.slice_size);
    // 1) 控制流：模式 + 计划
    control
        .write_all(&[MODE_MULTI])
        .await
        .map_err(classify_io)?;
    let mut plan = [0u8; PLAN_LEN];
    plan[0..8].copy_from_slice(&slice_size.to_le_bytes());
    plan[8..12].copy_from_slice(&(slice_count as u32).to_le_bytes());
    control.write_all(&plan).await.map_err(classify_io)?;

    // 2) 数据流任务：按切片逐条开流（索引即任务划分，流间互不等待）
    let size = offer.size;
    let received = Arc::new(AtomicU64::new(0));
    let done = Arc::new(SliceDone::new(slice_count));
    let mut tasks = FuturesUnordered::new();
    let mut next = 0;
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let receiving = async {
        while next < slice_count || !tasks.is_empty() {
            if next < slice_count && tasks.len() < streams {
                let mut stream = await_stream(conn.dial(), &mut tasks, &mut tick, || {
                    if let Some(progress) = progress.as_mut() {
                        progress(received.load(Ordering::Relaxed));
                    }
                })
                .await?;
                let index = next;
                stream
                    .write_all(&(index as u32).to_le_bytes())
                    .await
                    .map_err(classify_io)?;
                next += 1;
                let (temp2, received2, done2) =
                    (temp.to_path_buf(), received.clone(), done.clone());
                // 直接持有 future：返回错误或调用者取消时，所有切片立即一起释放。
                tasks.push(async move {
                    fetch_slice(
                        temp2,
                        stream,
                        index,
                        slice_size,
                        size,
                        SliceProgress {
                            received: received2,
                            done: done2,
                        },
                        tuning,
                    )
                    .await
                });
                continue;
            }
            tokio::select! {
                item = tasks.next(), if !tasks.is_empty() => {
                    if let Some(result) = item { result?; }
                }
                _ = tick.tick() => {
                    if let Some(progress) = progress.as_mut() {
                        progress(received.load(Ordering::Relaxed));
                    }
                }
            }
        }
        Ok::<(), OrchestrateError>(())
    };
    let hashing = hash_slices(
        temp.to_path_buf(),
        size,
        slice_size,
        slice_count,
        done.clone(),
        tuning,
    );
    let (_, local_sha_bytes) = tokio::try_join!(receiving, hashing)?;

    // 5) 裁决（诚实 rx：按本地核算提交）
    let mut verdict = [0u8; VERDICT_LEN];
    verdict[0] = VERDICT_OK;
    verdict[1..33].copy_from_slice(&local_sha_bytes);
    verdict[33..41].copy_from_slice(&received.load(Ordering::Relaxed).to_le_bytes());
    control.write_all(&verdict).await.map_err(classify_io)?;
    control.shutdown().await.map_err(classify_io)?;
    Ok((
        hex(&local_sha_bytes),
        received.load(Ordering::Relaxed) == offer.size,
    ))
}

/// 切片写入共享状态：累计进度与完成通知同属一次接收。
struct SliceProgress {
    received: Arc<AtomicU64>,
    done: Arc<SliceDone>,
}

/// rx：接收一个切片——读索引声明的区间，按偏移写盘，完成即置位。
async fn fetch_slice<S>(
    temp: std::path::PathBuf,
    mut stream: S,
    index: u64,
    slice_size: u64,
    size: u64,
    progress: SliceProgress,
    tuning: ConnectionTuning,
) -> Result<(), OrchestrateError>
where
    S: AsyncRead + Unpin + Send,
{
    let offset = index * slice_size;
    let len = (size - offset).min(slice_size);
    let mut dest = open_temp_rw(&temp).await?;
    dest.seek(io::SeekFrom::Start(offset))
        .await
        .map_err(classify_io)?;
    let mut remaining = len;
    let mut buf = vec![0u8; tuning.chunk.max(1)];
    while remaining > 0 {
        let want = (tuning.chunk.max(1) as u64).min(remaining) as usize;
        let n = stream.read(&mut buf[..want]).await.map_err(classify_io)?;
        if n == 0 {
            return Err(OrchestrateError::Protocol(
                "payload shorter than announced size".into(),
            ));
        }
        dest.write_all(&buf[..n]).await.map_err(classify_io)?;
        progress.received.fetch_add(n as u64, Ordering::Relaxed);
        remaining -= n as u64;
    }
    dest.flush().await.map_err(classify_io)?;
    progress.done.mark(index as usize);
    Ok(())
}

/// rx：按切片完成顺序读回并计算整文件 SHA-256（读回走页缓存）。
async fn hash_slices(
    path: std::path::PathBuf,
    size: u64,
    slice_size: u64,
    slice_count: u64,
    done: Arc<SliceDone>,
    tuning: ConnectionTuning,
) -> Result<[u8; 32], OrchestrateError> {
    let mut file = tokio::fs::File::open(&path).await.map_err(classify_io)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; tuning.chunk.max(1)];
    for index in 0..slice_count {
        done.wait_for(index as usize).await;
        file.seek(io::SeekFrom::Start(index * slice_size))
            .await
            .map_err(classify_io)?;
        let mut remaining = (size - index * slice_size).min(slice_size);
        while remaining > 0 {
            let want = (tuning.chunk.max(1) as u64).min(remaining) as usize;
            file.read_exact(&mut buf[..want])
                .await
                .map_err(classify_io)?;
            hasher.update(&buf[..want]);
            remaining -= want as u64;
        }
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&hasher.finalize());
    Ok(out)
}

/// 以读+写打开临时文件（收尾 sync 在 Windows 上需要写访问）。
async fn open_temp_rw(path: &std::path::Path) -> Result<tokio::fs::File, OrchestrateError> {
    tokio::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .await
        .map_err(classify_io)
}

/// tx：把本地文件经传输发给 rx（文件路径源，支持多流切片）。
pub async fn send_file<C>(
    conn: &mut C,
    path: &std::path::Path,
    progress: Option<Box<dyn FnMut(u64) + Send>>,
) -> Result<ServeOutcome, OrchestrateError>
where
    C: crate::transport::TransportConnection,
{
    let size = tokio::fs::metadata(path).await.map_err(classify_io)?.len();
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();
    crate::log_info!(path = %path.display().to_string(), size, "sending file");
    serve_file(
        conn,
        ServeOptions {
            kind: crate::transfer::Kind::File,
            name,
            size,
            source: PayloadSource::File(path.to_path_buf()),
            progress,
        },
    )
    .await
}

/// tx：把一段文本经传输发给 rx（`Kind::Text`，顺序流源 → 单流模式）。
pub async fn send_text<C>(
    conn: &mut C,
    text: &str,
    progress: Option<Box<dyn FnMut(u64) + Send>>,
) -> Result<ServeOutcome, OrchestrateError>
where
    C: crate::transport::TransportConnection,
{
    let bytes = text.as_bytes().to_vec();
    serve_file(
        conn,
        ServeOptions {
            kind: crate::transfer::Kind::Text,
            name: "message.txt".to_string(),
            size: bytes.len() as u64,
            source: PayloadSource::Stream(Box::new(std::io::Cursor::new(bytes))),
            progress,
        },
    )
    .await
}

/// 临时文件 RAII 清理：Drop 时删除（含 future 被取消/panic 的情况）。
struct TempGuard(std::path::PathBuf);

impl Drop for TempGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// rx：按 offer 收满字节数 → 临时文件 → 核算 → 原子改名。
pub async fn receive<C>(
    conn: &mut C,
    dest_dir: &std::path::Path,
    options: ReceiveOptions<'_>,
) -> Result<Receipt, OrchestrateError>
where
    C: crate::transport::TransportConnection,
{
    let mut tuning = conn.transfer_tuning();
    tuning.validate().map_err(OrchestrateError::Protocol)?;
    let offer = options.offer.ok_or_else(|| {
        OrchestrateError::Protocol(
            "payload size unknown (the sender never announced a transfer offer)".into(),
        )
    })?;
    offer.validate().map_err(OrchestrateError::Protocol)?;
    std::fs::create_dir_all(dest_dir).map_err(classify_io)?;
    // 唯一临时名（避免并发/重名）；create_temp 用 O_EXCL + 0600
    let mut rnd = [0u8; 8];
    {
        use rand_core::Rng;
        let mut rng = rand_core::UnwrapErr(getrandom::SysRng);
        rng.fill_bytes(&mut rnd);
    }
    let temp = dest_dir.join(format!(
        ".hermod_{:016x}.hermod_tmp",
        u64::from_le_bytes(rnd)
    ));
    // 占位创建后立即交出句柄；收发各自按需打开读+写句柄。
    drop(crate::transfer::create_temp(&temp).map_err(classify_io)?);
    // 守卫在函数返回/被取消时删除临时文件；成功改名后 forget 掉。
    let guard = TempGuard(temp.clone());

    let summary = Summary::new("rx", conn);
    let rtt = conn.rtt().await;
    let streams = resolve_streams(
        tuning,
        options.profile,
        conn.peer_addr(),
        rtt,
        offer.size,
        options.peer_multi_stream,
    );
    tuning.streams = Some(streams);
    let use_multi = streams > 1;
    let stream = conn
        .dial()
        .await
        .map_err(|e| OrchestrateError::Transport(e.to_string()))?;
    let (sha256, verified) = if use_multi {
        fetch_multi(conn, stream, offer, &temp, options.progress, tuning).await?
    } else {
        fetch_single(stream, offer, &temp, options.progress, tuning).await?
    };
    // 落盘完整后 sync（Windows 的 FlushFileBuffers 需要写访问 → 读+写句柄）
    let file = open_temp_rw(&temp).await?;
    file.sync_all().await.map_err(classify_io)?;
    drop(file);

    if !verified {
        crate::log_warn!(
            "size mismatch vs announced offer, removing temp file {}",
            temp.display()
        );
        let _ = std::fs::remove_file(&temp);
        return Err(OrchestrateError::DigestMismatch);
    }

    // 校验通过才原子改名（safe_destination_path 负责剥目录分量 + 去重）
    let final_path = crate::transfer::safe_destination_path(dest_dir, &offer.name)?;
    crate::log_info!(
        from = %temp.display().to_string(),
        to = %final_path.display().to_string(),
        "verified, renaming"
    );
    std::fs::rename(&temp, &final_path).map_err(classify_io)?;
    std::mem::forget(guard);
    summary.finish(
        if use_multi { "multi" } else { "single" },
        streams,
        offer.size,
    );
    Ok(Receipt {
        path: final_path,
        sha256,
        size: offer.size,
        kind: offer.kind,
    })
}
