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

    // 1) 生成并打印传输码（stdout 只放这个）
    let mut rng = rand_core::UnwrapErr(getrandom::SysRng);
    let (_channel_id, code) = wordlist::generate_transfer_code(&mut rng, args.words);
    println!("{code}");

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
    verbosity::success("tunnel established");

    // 3) 按输入类型发送
    let quiet = verbosity::is_quiet();
    let (kind, _name) = transfer::classify_input(&args.input, crate::tty::stdin_is_piped());

    match kind {
        Kind::File => {
            let path = Path::new(&args.input);
            let total = std::fs::metadata(path).map_err(util::boxed)?.len();
            let pb = progress::bar(total, quiet);
            let (tick_tx, tick_rx) = mpsc::channel(1);
            let pb2 = pb.clone();
            let progress: Option<Box<dyn FnMut(u64) + Send>> = Some(Box::new(move |n| {
                pb2.set_position(n);
                let _ = tick_tx.try_send(());
            }));
            let fut = orchestrate::send_file(&mut tx.conn, tx.token, path, progress);
            let out = run_with_idle(fut, tick_rx).await?;
            pb.finish_and_clear();
            finish(out.verified, total, &out.sha256)?;
        }
        Kind::Text => {
            let total = args.input.len() as u64;
            let pb = progress::bar(total, quiet);
            let (tick_tx, tick_rx) = mpsc::channel(1);
            let pb2 = pb.clone();
            let progress: Option<Box<dyn FnMut(u64) + Send>> = Some(Box::new(move |n| {
                pb2.set_position(n);
                let _ = tick_tx.try_send(());
            }));
            let fut = orchestrate::send_text(&mut tx.conn, tx.token, &args.input, progress);
            let out = run_with_idle(fut, tick_rx).await?;
            pb.finish_and_clear();
            finish(out.verified, total, &out.sha256)?;
        }
        Kind::Stream => {
            let mut data = Vec::new();
            std::io::stdin().read_to_end(&mut data).map_err(util::boxed)?;
            let size = data.len() as u64;
            let pb = progress::bar(size, quiet);
            let (tick_tx, tick_rx) = mpsc::channel(1);
            let pb2 = pb.clone();
            let progress: Option<Box<dyn FnMut(u64) + Send>> = Some(Box::new(move |n| {
                pb2.set_position(n);
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
            let out = run_with_idle(fut, tick_rx).await?;
            pb.finish_and_clear();
            finish(out.verified, size, &out.sha256)?;
        }
    }
    Ok(())
}

fn finish(verified: bool, size: u64, sha256: &str) -> Result<(), Box<dyn std::error::Error>> {
    if !verified {
        return Err("receiver 的摘要未被接受（digest mismatch）".into());
    }
    verbosity::success(format!("sent {size} bytes, sha256={sha256}"));
    Ok(())
}

async fn run_with_idle<F, T>(
    fut: F,
    ticks: mpsc::Receiver<()>,
) -> Result<T, Box<dyn std::error::Error>>
where
    F: std::future::Future<Output = Result<T, orchestrate::OrchestrateError>>,
{
    match progress::with_idle_timeout(fut, ticks, IDLE).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(Box::new(e)),
        Err(msg) => Err(msg.into()),
    }
}
