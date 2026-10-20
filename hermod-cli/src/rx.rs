//! `hermod rx`：用传输码建立连接，拉取文件并落盘。

use crate::cli::RxArgs;
use crate::{progress, util, verbosity};
use hermod_core::TransportConnection;
use hermod_core::config;
use hermod_core::orchestrate;
use hermod_core::session::{self, SessionOptions};
use hermod_core::transport::{CongestionControl, TransportKind};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;

const IDLE: Duration = Duration::from_secs(30);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(660);

/// 执行 `rx`。
pub async fn run(
    args: RxArgs,
    abort_slot: util::AbortSlot,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut cfg = config::load().unwrap_or_default();
    if let Some(s) = &args.server {
        cfg.server_url = config::normalize_server_url(s).map_err(util::boxed)?;
    }

    verbosity::debug(format!("server_url={}", cfg.server_url));
    let opts = SessionOptions {
        mtu: Some(args.wg_mtu),
        udp_bind: Some(util::parse_listen(&args.listen)?),
        transport: TransportKind::parse(&args.transport_opts.transport).unwrap_or_default(),
        quic_congestion: CongestionControl::parse(&args.transport_opts.quic_cc).unwrap_or_default(),
        quic_mtu: Some(args.transport_opts.quic_mtu),
    };
    let mut rx = tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        session::receiver_session_with(&cfg, &args.code, &opts),
    )
    .await
    .map_err(|_| "handshake timed out")?
    .map_err(util::boxed)?;
    // 连接已建立：把中止句柄交给 Ctrl+C 处理器（取消时对端立刻收到 RST）。
    if let Ok(mut s) = abort_slot.lock() {
        *s = Some(rx.conn.abort_handle());
    }
    verbosity::status("Establishing P2P connection...");

    let dest = args.destination.unwrap_or_else(|| ".".to_string());
    std::fs::create_dir_all(&dest).map_err(util::boxed)?;

    let pb = progress::spinner(verbosity::is_quiet());
    let (tick_tx, tick_rx) = mpsc::channel(1);
    let received = Arc::new(AtomicU64::new(0));
    let pb2 = pb.clone();
    let got2 = received.clone();
    let mut speed = progress::Speed::new();
    let mut on_progress = move |n: u64| {
        let rate = speed.update(n);
        pb2.set_message(format!(
            "{} ({}/s)",
            indicatif::HumanBytes(n),
            indicatif::HumanBytes(rate as u64)
        ));
        got2.store(n, Ordering::Relaxed);
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
        Err(msg) => {
            return Err(format!(
                "{msg} (received {} bytes)",
                received.load(Ordering::Relaxed)
            )
            .into());
        }
    };
    pb.finish_and_clear();
    // 优雅关闭 rx 侧（排空后驱动退出）
    rx.conn.shutdown().await.map_err(util::boxed)?;

    // 对齐 Go：状态打到 stderr（--quiet 抑制）；sha256 放在 info 日志。
    verbosity::status(format!("Saved to {}", receipt.path.display()));
    verbosity::info(format!(
        "sha256: {}, bytes: {}",
        receipt.sha256, receipt.size
    ));
    verbosity::status("Transfer complete.");
    Ok(())
}
