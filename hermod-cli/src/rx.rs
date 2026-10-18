//! `hermod rx`：用传输码建立连接，拉取文件并落盘。

use crate::cli::RxArgs;
use crate::{progress, util, verbosity};
use hermod_core::config;
use hermod_core::orchestrate;
use hermod_core::session::{self, SessionOptions};
use std::path::Path;
use std::time::Duration;
use tokio::sync::mpsc;

const IDLE: Duration = Duration::from_secs(30);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(660);

/// 执行 `rx`。
pub async fn run(args: RxArgs) -> Result<(), Box<dyn std::error::Error>> {
    let mut cfg = config::load().unwrap_or_default();
    if let Some(s) = &args.server {
        cfg.server_url = config::normalize_server_url(s).map_err(util::boxed)?;
    }

    verbosity::debug(format!("server_url={}", cfg.server_url));
    let opts = SessionOptions {
        mtu: Some(args.wg_mtu),
        udp_bind: Some(util::parse_listen(&args.listen)?),
    };
    let mut rx = tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        session::receiver_session_with(&cfg, &args.code, &opts),
    )
    .await
    .map_err(|_| "handshake timed out")?
    .map_err(util::boxed)?;
    verbosity::success("tunnel established");

    let dest = args.destination.unwrap_or_else(|| ".".to_string());
    std::fs::create_dir_all(&dest).map_err(util::boxed)?;

    let pb = progress::spinner(verbosity::is_quiet());
    let (tick_tx, tick_rx) = mpsc::channel(1);
    let pb2 = pb.clone();
    let mut on_progress = move |n: u64| {
        pb2.set_message(format!("{n} bytes"));
        let _ = tick_tx.try_send(());
    };
    let fut = orchestrate::receive(
        &mut rx.conn,
        &rx.token,
        Path::new(&dest),
        Some(&mut on_progress),
    );
    let receipt = match progress::with_idle_timeout(fut, tick_rx, IDLE).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => return Err(Box::new(e)),
        Err(msg) => return Err(msg.into()),
    };
    pb.finish_and_clear();
    // 优雅关闭 rx 侧（排空后驱动退出）
    rx.conn.shutdown().await.map_err(util::boxed)?;

    // stdout：最终路径 + SHA-256（便于脚本取用）
    println!("{}", receipt.path.display());
    println!("{}", receipt.sha256);
    verbosity::success(format!(
        "received {} bytes → {}",
        receipt.size,
        receipt.path.display()
    ));
    Ok(())
}
