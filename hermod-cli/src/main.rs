//! Hermod CLI 入口 —— `hermod-core` 的薄封装（D17）。

mod cli;
mod progress;
mod rx;
mod serve;
mod tty;
mod trust;
mod tx;
mod util;
mod verbosity;

use clap::Parser;
use verbosity::Level;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let args = cli::Cli::parse();
    let level = match args.verbose.as_str() {
        "error" => Level::Error,
        "warning" => Level::Warning,
        "info" => Level::Info,
        "debug" => Level::Debug,
        _ => Level::None,
    };
    verbosity::set(level, args.quiet);
    // 把 core 的日志接到 CLI 的 verbosity（级别与 --verbose 一一对应）
    // 结构化日志：对标 Go 的 slog，级别由 --verbose 决定。
    // 注意：--quiet 与 --verbose 正交（Go 语义）：-q 只抑制状态输出，不关日志。
    let filter = match level {
        Level::None => tracing_subscriber::EnvFilter::new("off"),
        Level::Error => tracing_subscriber::EnvFilter::new("error"),
        Level::Warning => tracing_subscriber::EnvFilter::new("warn"),
        Level::Info => tracing_subscriber::EnvFilter::new("info"),
        Level::Debug => tracing_subscriber::EnvFilter::new("debug"),
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .with_level(true)
        .init();

    // Ctrl+C 取消：连接建立后 tx/rx 会把中止句柄放进这个槽；
    // 收到 Ctrl+C 时取出并 abort()，驱动会 RST 掉连接，对端立即结束。
    let abort_slot: util::AbortSlot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let slot_for_ctrl = abort_slot.clone();
    let result = tokio::select! {
        r = run(args, abort_slot) => r,
        _ = tokio::signal::ctrl_c() => {
            if let Some(h) = slot_for_ctrl.lock().ok().and_then(|mut s| s.take()) {
                h.abort();
                // 留一点时间让驱动把 RST 发出去。
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
            verbosity::error("interrupted");
            return std::process::ExitCode::from(130);
        }
    };

    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            verbosity::error(e.to_string());
            std::process::ExitCode::from(1)
        }
    }
}

async fn run(
    args: cli::Cli,
    abort_slot: util::AbortSlot,
) -> Result<(), Box<dyn std::error::Error>> {
    match args.cmd {
        cli::Command::Tx(a) => tx::run(a, abort_slot).await,
        cli::Command::Rx(a) => rx::run(a, abort_slot).await,
        cli::Command::Trust(a) => trust::run(a).await,
        cli::Command::Serve => serve::run().await,
    }
}
