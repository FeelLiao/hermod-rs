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
use std::io::Read;
use std::path::Path;
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
    let mut cfg = config::load().unwrap_or_default();
    if let Some(s) = &args.server {
        cfg.server_url = config::normalize_server_url(s).map_err(util::boxed)?;
    }

    // 0) 先确认 server 已信任，避免打印一个用不了的传输码
    hermod_core::session::expected_spki(&cfg).map_err(util::boxed)?;

    // 1) 生成传输码，频道创建成功后才在回调中打印到 stderr。
    let mut rng = rand_core::UnwrapErr(getrandom::SysRng);
    let (_channel_id, code) = wordlist::generate_transfer_code(&mut rng, usize::from(args.words));

    // 2) 建会话
    verbosity::debug(format!("server_url={}", cfg.server_url));
    let opts = args.network.options(&args.transport, false)?;
    let mut tx = tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        session::sender_session_with_ready(&cfg, &code, &opts, || {
            eprintln!("Transfer code: {code}")
        }),
    )
    .await
    .map_err(|_| "handshake timed out")?
    .map_err(util::boxed)?;
    // 连接已建立：把中止句柄交给 Ctrl+C 处理器（取消时对端立刻收到 RST）。
    if let Ok(mut s) = abort_slot.lock() {
        *s = Some(tx.conn.abort_handle());
    }
    verbosity::status("Establishing P2P connection...");

    // 3) 按输入类型发送
    let quiet = verbosity::is_quiet();
    let (kind, _name) = transfer::classify_input(&args.input, crate::tty::stdin_is_piped());

    match kind {
        Kind::File => {
            let path = Path::new(&args.input);
            let total = std::fs::metadata(path).map_err(util::boxed)?.len();
            let net = tx.conn.network_stats();
            let pb = if net.is_some() {
                progress::bar_net(total, quiet)
            } else {
                progress::bar(total, quiet)
            };
            let _net = spawn_net_progress(&pb, net, total);
            let (tick_tx, tick_rx) = mpsc::channel(1);
            let sent = Arc::new(AtomicU64::new(0));
            let pb2 = pb.clone();
            let sent2 = sent.clone();
            let progress: Option<Box<dyn FnMut(u64) + Send>> = Some(Box::new(move |n| {
                pb2.set_position(n);
                sent2.store(n, Ordering::Relaxed);
                let _ = tick_tx.try_send(());
            }));
            let fut = orchestrate::send_file_with_limit(
                &mut tx.conn,
                tx.token,
                path,
                progress,
                args.network.limit_mbps,
            );
            let out = run_with_idle(fut, tick_rx, sent).await?;
            pb.finish_and_clear();
            finish_and_shutdown(&mut tx, out.verified, total, &out.sha256).await?;
        }
        Kind::Text => {
            let total = args.input.len() as u64;
            let net = tx.conn.network_stats();
            let pb = if net.is_some() {
                progress::bar_net(total, quiet)
            } else {
                progress::bar(total, quiet)
            };
            let _net = spawn_net_progress(&pb, net, total);
            let (tick_tx, tick_rx) = mpsc::channel(1);
            let sent = Arc::new(AtomicU64::new(0));
            let pb2 = pb.clone();
            let sent2 = sent.clone();
            let progress: Option<Box<dyn FnMut(u64) + Send>> = Some(Box::new(move |n| {
                pb2.set_position(n);
                sent2.store(n, Ordering::Relaxed);
                let _ = tick_tx.try_send(());
            }));
            let fut = orchestrate::send_text_with_limit(
                &mut tx.conn,
                tx.token,
                &args.input,
                progress,
                args.network.limit_mbps,
            );
            let out = run_with_idle(fut, tick_rx, sent).await?;
            pb.finish_and_clear();
            finish_and_shutdown(&mut tx, out.verified, total, &out.sha256).await?;
        }
        Kind::Stream => {
            let mut data = Vec::new();
            std::io::stdin()
                .read_to_end(&mut data)
                .map_err(util::boxed)?;
            let size = data.len() as u64;
            let net = tx.conn.network_stats();
            let pb = if net.is_some() {
                progress::bar_net(size, quiet)
            } else {
                progress::bar(size, quiet)
            };
            let _net = spawn_net_progress(&pb, net, size);
            let (tick_tx, tick_rx) = mpsc::channel(1);
            let sent = Arc::new(AtomicU64::new(0));
            let pb2 = pb.clone();
            let sent2 = sent.clone();
            let progress: Option<Box<dyn FnMut(u64) + Send>> = Some(Box::new(move |n| {
                pb2.set_position(n);
                sent2.store(n, Ordering::Relaxed);
                let _ = tick_tx.try_send(());
            }));
            let serve_opts = ServeOptions {
                token: tx.token,
                kind: Kind::Stream,
                name: "stdin.bin".into(),
                size,
                body: Box::new(std::io::Cursor::new(data)),
                progress,
            };
            let fut = orchestrate::serve_file_with_limit(
                &mut tx.conn,
                serve_opts,
                args.network.limit_mbps,
            );
            let out = run_with_idle(fut, tick_rx, sent).await?;
            pb.finish_and_clear();
            finish_and_shutdown(&mut tx, out.verified, size, &out.sha256).await?;
        }
    }
    Ok(())
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
