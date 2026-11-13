//! `hermod tx`：生成传输码，把文件 / 文本 / stdin 经隧道发给 rx。

use crate::cli::TxArgs;
use crate::{progress, util, verbosity};
use hermod_core::TransportConnection;
use hermod_core::config;
use hermod_core::crypto::wordlist;
use hermod_core::orchestrate;
use hermod_core::orchestrate::ServeOptions;
use hermod_core::session;
use hermod_core::transfer::{self, Kind};
use hermod_core::transport::NetworkStatsHandle;
use std::io::Seek;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(660);

/// 执行 `tx`。
pub async fn run(
    args: TxArgs,
    mut opts: session::SessionOptions,
    abort_slot: util::AbortSlot,
) -> Result<(), Box<dyn std::error::Error>> {
    let cfg = config::load().map_err(util::boxed)?;

    // 0) 先确认 server 已信任，避免打印一个用不了的传输码
    hermod_core::session::expected_spki(&cfg).map_err(util::boxed)?;

    // 先准备输入并确定大小；文件源用路径（可 seek，支持多流切片），
    // 顺序源（stdin/文本）保留流形状走单流。
    let (kind, name) = transfer::classify_input(&args.input, crate::tty::stdin_is_piped());
    let mut staged = None;
    let source: orchestrate::PayloadSource;
    let size;
    let name = match kind {
        Kind::File => {
            let path = PathBuf::from(&args.input);
            let metadata = tokio::fs::metadata(&path).await?;
            if !metadata.is_file() {
                return Err("Select a regular file".into());
            }
            size = metadata.len();
            source = orchestrate::PayloadSource::File(path);
            name
        }
        Kind::Text => {
            size = args.input.len() as u64;
            source = orchestrate::PayloadSource::Stream(Box::new(std::io::Cursor::new(
                args.input.into_bytes(),
            )));
            "message.txt".into()
        }
        Kind::Stream => {
            // 临时文件守卫随阻塞任务的返回值移动；取消后返回值被丢弃也能清理。
            let (temporary, file, length) =
                tokio::task::spawn_blocking(|| -> std::io::Result<_> {
                    let mut random = [0u8; 16];
                    getrandom::fill(&mut random).map_err(std::io::Error::other)?;
                    let path = std::env::temp_dir()
                        .join(format!("hermod-stdin-{}", transfer::hex(&random)));
                    let mut options = std::fs::OpenOptions::new();
                    options.read(true).write(true).create_new(true);
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::OpenOptionsExt;
                        options.mode(0o600);
                    }
                    let mut file = options.open(&path)?;
                    let temporary = StagedInput(path);
                    // Linux 发布版只保留已打开句柄，异常退出也不会留下标准输入内容。
                    #[cfg(unix)]
                    std::fs::remove_file(&temporary.0)?;
                    let length = std::io::copy(&mut std::io::stdin().lock(), &mut file)?;
                    file.rewind()?;
                    Ok((temporary, file, length))
                })
                .await??;
            staged = Some(temporary);
            size = length;
            source = orchestrate::PayloadSource::Stream(Box::new(tokio::fs::File::from_std(file)));
            "stdin.bin".into()
        }
    };
    let offer = transfer::TransferOffer { kind, name, size };
    offer.validate()?;
    opts.offer = Some(offer.clone());
    // 可 seek 的文件源经 bundle 声明，接收端才可能选择多流切片模式。
    opts.multi_stream_capable = matches!(&source, orchestrate::PayloadSource::File(_));
    let mut rng = rand_core::UnwrapErr(getrandom::SysRng);
    let (_, code) = wordlist::generate_transfer_code(&mut rng, 3);
    let device_dir = config::dir();
    let (identity, contacts, _send_lease) = tokio::task::spawn_blocking(move || {
        let identity = hermod_core::identity::Identity::load_or_create(&device_dir)?;
        let contacts = hermod_core::contacts::Contacts::open(&device_dir)?;
        let lease = contacts.acquire_send_lease()?;
        Ok::<_, hermod_core::identity::Error>((identity, contacts, lease))
    })
    .await??;
    verbosity::status(format!("Device ID: {}", identity.device_id()));
    let mut tx = tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        session::sender_device_session(&cfg, &code, &opts, &identity, &contacts, args.pair, || {
            eprintln!("Transfer code: {code}")
        }),
    )
    .await
    .map_err(|_| "handshake timed out")?
    .map_err(util::boxed)?;
    if args.pair {
        verbosity::status("Device pairing confirmed.");
    }
    verbosity::status("Receiver authenticated; transfer code is no longer active.");
    if let Ok(mut s) = abort_slot.lock() {
        *s = Some(tx.conn.abort_handle());
    }
    verbosity::status("Establishing P2P connection...");
    let net = tx.conn.network_stats();
    let pb = if net.is_some() {
        progress::bar_net(size, verbosity::is_quiet())
    } else {
        progress::bar(size, verbosity::is_quiet())
    };
    let _net = spawn_net_progress(&pb, net, size);
    let (tick_tx, tick_rx) = mpsc::channel(1);
    let sent = Arc::new(AtomicU64::new(0));
    let pb2 = pb.clone();
    let sent2 = sent.clone();
    let progress = Some(Box::new(move |n| {
        pb2.set_position(n);
        // 多流定时回调可能重复同一累计值；只有真实字节进展才重置看门狗。
        if n > sent2.fetch_max(n, Ordering::Relaxed) {
            let _ = tick_tx.try_send(());
        }
    }) as Box<dyn FnMut(u64) + Send>);
    let out = run_with_idle(
        orchestrate::serve_file(
            &mut tx.conn,
            ServeOptions {
                kind,
                name: offer.name,
                size,
                source,
                progress,
            },
        ),
        tick_rx,
        sent,
        opts.tuning.idle_timeout,
    )
    .await?;
    pb.finish_and_clear();
    finish_and_shutdown(&mut tx, out.verified, size, &out.sha256).await?;
    drop(staged);
    Ok(())
}

/// 已知长度的标准输入私有缓存。
struct StagedInput(PathBuf);
impl Drop for StagedInput {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// 网络统计后台任务的 RAII 守卫：离开作用域时 abort。
struct NetGuard(Option<tokio::task::JoinHandle<()>>);

impl Drop for NetGuard {
    fn drop(&mut self) {
        if let Some(task) = self.0.take() {
            task.abort();
        }
    }
}

/// 每秒读取一次 QUIC 网络统计，把 `7.2 MiB/s (5m)` 写进进度条消息。
fn spawn_net_progress(
    pb: &indicatif::ProgressBar,
    handle: Option<NetworkStatsHandle>,
    total: u64,
) -> NetGuard {
    let Some(handle) = handle else {
        return NetGuard(None);
    };
    let pb = pb.clone();
    let task = tokio::spawn(async move {
        let mut last = handle.snapshot();
        let mut last_t = Instant::now();
        let mut rate = 0.0f64;
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let now = Instant::now();
            let stats = handle.snapshot();
            let dt = now.duration_since(last_t).as_secs_f64().max(0.001);
            let delta = stats.wire_bytes.saturating_sub(last.wire_bytes) as f64;
            let instant = delta / dt;
            rate = if rate == 0.0 {
                instant
            } else {
                0.3 * instant + 0.7 * rate
            };
            last = stats;
            last_t = now;

            let remaining = total.saturating_sub(pb.position());
            let eta = if remaining == 0 {
                "finishing".to_string()
            } else if rate > 1024.0 {
                progress::format_eta(remaining as f64 / rate)
            } else {
                "--".to_string()
            };
            pb.set_message(format!("{} ({})", progress::format_rate(rate), eta));
        }
    });
    NetGuard(Some(task))
}

async fn finish_and_shutdown(
    tx: &mut hermod_core::session::TxSession,
    verified: bool,
    size: u64,
    sha256: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // 等驱动把最后的裁决/尾数据送达对端，再返回（否则进程退出会 abort 驱动）
    tx.conn.shutdown().await.map_err(util::boxed)?;
    finish(verified, size, sha256)
}

fn finish(verified: bool, size: u64, sha256: &str) -> Result<(), Box<dyn std::error::Error>> {
    if !verified {
        return Err("receiver digest not accepted (digest mismatch)".into());
    }
    verbosity::info(format!("sha256: {sha256}, bytes: {size}"));
    verbosity::status("Transfer complete.");
    Ok(())
}

async fn run_with_idle<F, T>(
    fut: F,
    ticks: mpsc::Receiver<()>,
    sent: Arc<AtomicU64>,
    idle_timeout: Duration,
) -> Result<T, Box<dyn std::error::Error>>
where
    F: std::future::Future<Output = Result<T, orchestrate::OrchestrateError>>,
{
    match progress::with_idle_timeout(fut, ticks, idle_timeout).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(Box::new(e)),
        Err(msg) => Err(format!(
            "{msg} ({} bytes written; for large files, check the peer; \
             try --mtu 1200 to rule out path MTU issues)",
            sent.load(Ordering::Relaxed)
        )
        .into()),
    }
}
