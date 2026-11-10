//! `hermod rx`：用传输码建立连接，拉取文件并落盘。

use crate::cli::RxArgs;
use crate::{progress, util, verbosity};
use hermod_core::TransportConnection;
use hermod_core::config;
use hermod_core::orchestrate;
use hermod_core::session;
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
    let mut cfg = config::load().map_err(util::boxed)?;
    if let Some(s) = &args.server {
        cfg.server_url = config::normalize_server_url(s).map_err(util::boxed)?;
    }

    verbosity::debug(format!("server_url={}", cfg.server_url));
    let opts = args.network.options(true)?;
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
    let on_progress = move |n: u64| {
        let rate = speed.update(n);
        pb2.set_position(n);
        pb2.set_message(format!(
            "{} ({}/s)",
            indicatif::HumanBytes(n),
            indicatif::HumanBytes(rate as u64)
        ));
        got2.store(n, Ordering::Relaxed);
        let _ = tick_tx.try_send(());
    };
    let mut on_progress = on_progress;
    // 协议 v2：元数据来自端点交换里的 offer（进度条总长已知）
    if let Some(offer) = &rx.offer {
        pb.set_length(offer.size);
        pb.set_style(
            indicatif::ProgressStyle::with_template(
                "{bar:40} {bytes}/{total_bytes} {bytes_per_sec} ({eta})",
            )
            .unwrap(),
        );
    }
    let fut = orchestrate::receive(
        &mut rx.conn,
        Path::new(&dest),
        orchestrate::ReceiveOptions {
            offer: rx.offer.as_ref(),
            progress: Some(&mut on_progress),
            profile: opts.profile,
            peer_multi_stream: rx.peer_multi_stream,
        },
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
