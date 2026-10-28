//! `hermod tx`：生成传输码，把文件 / 文本 / stdin 经隧道发给 rx。

use crate::cli::TxArgs;
use crate::{progress, util, verbosity};
use hermod_core::TransportConnection;
use hermod_core::config;
use hermod_core::crypto::wordlist;
use hermod_core::orchestrate;
use hermod_core::session;
use hermod_core::transfer::{self, Kind};
use hermod_core::transport::NetworkStatsHandle;
use hermod_core::transport::http::server::ServeOptions;
use std::io::Seek;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

const IDLE: Duration = Duration::from_secs(30);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(660);

/// 执行 `tx`。
pub async fn run(
    args: TxArgs,
    abort_slot: util::AbortSlot,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut cfg = config::load().map_err(util::boxed)?;
    if let Some(s) = &args.server {
        cfg.server_url = config::normalize_server_url(s).map_err(util::boxed)?;
    }

    // 0) 先确认 server 已信任，避免打印一个用不了的传输码
    hermod_core::session::expected_spki(&cfg).map_err(util::boxed)?;

    let mut opts = args.network.options(&args.transport, false)?;

    // 先准备输入并确定大小；打开的同一个文件句柄贯穿预告和发送。
    let (kind, name) = transfer::classify_input(&args.input, crate::tty::stdin_is_piped());
    let mut staged = None;
    let body: Box<dyn tokio::io::AsyncRead + Unpin + Send>;
    let size;
    let name = match kind {
        Kind::File => {
            let file = tokio::fs::File::open(Path::new(&args.input)).await?;
            let metadata = file.metadata().await?;
            if !metadata.is_file() {
                return Err("Select a regular file".into());
            }
            size = metadata.len();
            body = Box::new(file);
            name
        }
        Kind::Text => {
            size = args.input.len() as u64;
            body = Box::new(std::io::Cursor::new(args.input.into_bytes()));
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
            body = Box::new(tokio::fs::File::from_std(file));
            "stdin.bin".into()
        }
    };
    let offer = transfer::TransferOffer { kind, name, size };
    offer.validate()?;
    opts.offer = Some(offer.clone());
    let mut rng = rand_core::UnwrapErr(getrandom::SysRng);
    let (_, code) = wordlist::generate_transfer_code(&mut rng, usize::from(args.words));
    let mut tx = tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        session::sender_session_with_ready(&cfg, &code, &opts, || {
            eprintln!("Transfer code: {code}")
        }),
    )
    .await
    .map_err(|_| "handshake timed out")?
    .map_err(util::boxed)?;
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
        sent2.store(n, Ordering::Relaxed);
        let _ = tick_tx.try_send(());
    }) as Box<dyn FnMut(u64) + Send>);
    let out = run_with_idle(
        orchestrate::serve_file_with_limit(
            &mut tx.conn,
            ServeOptions {
                token: tx.token,
                kind,
                name: offer.name,
                size,
                body,
                progress,
            },
            args.network.limit_mbps,
        ),
        tick_rx,
        sent,
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
) -> Result<T, Box<dyn std::error::Error>>
where
    F: std::future::Future<Output = Result<T, orchestrate::OrchestrateError>>,
{
    match progress::with_idle_timeout(fut, ticks, IDLE).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(Box::new(e)),
        Err(msg) => Err(format!(
            "{msg} ({} bytes written; for large files, check the peer; \
             try --transport wireguard --mtu 1200 or --transport quic --mtu 1200 to rule out path MTU)",
            sent.load(Ordering::Relaxed)
        )
        .into()),
    }
}
