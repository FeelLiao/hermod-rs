//! `hermod tx`：生成传输码，把文件 / 文本 / stdin 经隧道发给 rx。

use crate::cli::TxArgs;
use crate::{progress, util, verbosity};
use hermod_core::config;
use hermod_core::crypto::wordlist;
use hermod_core::orchestrate;
use hermod_core::session::{self, SessionOptions};
use hermod_core::transfer::{self, Kind};
use hermod_core::transport::wireguard::http::server::ServeOptions;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;

const IDLE: Duration = Duration::from_secs(30);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(660);

/// 执行 `tx`。
pub async fn run(args: TxArgs) -> Result<(), Box<dyn std::error::Error>> {
    let mut cfg = config::load().unwrap_or_default();
    if let Some(s) = &args.server {
        cfg.server_url = config::normalize_server_url(s).map_err(util::boxed)?;
    }

    // 0) 先确认 server 已信任，避免打印一个用不了的传输码
    hermod_core::session::expected_spki(&cfg).map_err(util::boxed)?;

    // 1) 生成并打印传输码（stdout 只放这个）
    let mut rng = rand_core::UnwrapErr(getrandom::SysRng);
    let (_channel_id, code) = wordlist::generate_transfer_code(&mut rng, args.words);
    // 对齐 Go：传输码打到 stderr，格式 "Transfer code: <code>"；不受 --quiet 抑制。
    eprintln!("Transfer code: {code}");

    // 2) 建会话
    verbosity::debug(format!("server_url={}", cfg.server_url));
    let opts = SessionOptions {
        mtu: Some(args.wg_mtu),
        udp_bind: Some(util::parse_listen(&args.listen)?),
    };
    let mut tx = tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        session::sender_session_with(&cfg, &code, &opts),
    )
    .await
    .map_err(|_| "handshake timed out")?
    .map_err(util::boxed)?;
    verbosity::status("Establishing P2P connection...");

    // 3) 按输入类型发送
    let quiet = verbosity::is_quiet();
    let (kind, _name) = transfer::classify_input(&args.input, crate::tty::stdin_is_piped());

    match kind {
        Kind::File => {
            let path = Path::new(&args.input);
            let total = std::fs::metadata(path).map_err(util::boxed)?.len();
            let pb = progress::bar(total, quiet);
            let (tick_tx, tick_rx) = mpsc::channel(1);
            let sent = Arc::new(AtomicU64::new(0));
            let pb2 = pb.clone();
            let sent2 = sent.clone();
            let progress: Option<Box<dyn FnMut(u64) + Send>> = Some(Box::new(move |n| {
                pb2.set_position(n);
                sent2.store(n, Ordering::Relaxed);
                let _ = tick_tx.try_send(());
            }));
            let fut = orchestrate::send_file(&mut tx.conn, tx.token, path, progress);
            let out = run_with_idle(fut, tick_rx, sent).await?;
            pb.finish_and_clear();
            finish_and_shutdown(&mut tx, out.verified, total, &out.sha256).await?;
        }
        Kind::Text => {
            let total = args.input.len() as u64;
            let pb = progress::bar(total, quiet);
            let (tick_tx, tick_rx) = mpsc::channel(1);
            let sent = Arc::new(AtomicU64::new(0));
            let pb2 = pb.clone();
            let sent2 = sent.clone();
            let progress: Option<Box<dyn FnMut(u64) + Send>> = Some(Box::new(move |n| {
                pb2.set_position(n);
                sent2.store(n, Ordering::Relaxed);
                let _ = tick_tx.try_send(());
            }));
            let fut = orchestrate::send_text(&mut tx.conn, tx.token, &args.input, progress);
            let out = run_with_idle(fut, tick_rx, sent).await?;
            pb.finish_and_clear();
            finish_and_shutdown(&mut tx, out.verified, total, &out.sha256).await?;
        }
        Kind::Stream => {
            let mut data = Vec::new();
            std::io::stdin().read_to_end(&mut data).map_err(util::boxed)?;
            let size = data.len() as u64;
            let pb = progress::bar(size, quiet);
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
                name: String::new(),
                size,
                body: Box::new(std::io::Cursor::new(data)),
                progress,
            };
            let fut = orchestrate::serve_file(&mut tx.conn, serve_opts);
            let out = run_with_idle(fut, tick_rx, sent).await?;
            pb.finish_and_clear();
            finish_and_shutdown(&mut tx, out.verified, size, &out.sha256).await?;
        }
    }
    Ok(())
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
            "{msg} ({} bytes written; for large files, check the peer; try --wg-mtu 1200 to rule out path MTU)",
            sent.load(Ordering::Relaxed)
        )
        .into()),
    }
}
