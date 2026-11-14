//! Hermod CLI 入口 —— `hermod-core` 的薄封装（D17）。

mod cli;
mod device;
mod progress;
mod rx;
mod serve;
mod trust;
mod tty;
mod tx;
mod util;
mod verbosity;
#[cfg(windows)]
mod windows;

use clap::{CommandFactory, Parser};
use verbosity::Level;

fn main() -> std::process::ExitCode {
    #[cfg(windows)]
    if let Err(error) = hermod_core::identity::install_storage_permissions(std::sync::Arc::new(
        windows::NativePermissions,
    )) {
        eprintln!("Failed to initialize private storage permissions: {error}");
        return std::process::ExitCode::FAILURE;
    }
    // 保持既有大栈工作线程行为，避免改变平台上的传输执行环境。
    match std::thread::Builder::new()
        .name("hermod-cli".into())
        .stack_size(64 * 1024 * 1024)
        .spawn(run_cli)
    {
        Ok(worker) => match worker.join() {
            Ok(exit) => exit,
            Err(_) => std::process::ExitCode::from(101),
        },
        Err(_) => {
            eprintln!("Failed to start CLI worker thread");
            std::process::ExitCode::from(101)
        }
    }
}

#[tokio::main]
async fn run_cli() -> std::process::ExitCode {
    let args = cli::Cli::parse();
    if let Err(message) = args.validate() {
        cli::Cli::command()
            .error(clap::error::ErrorKind::ValueValidation, message)
            .exit();
    }
    let level = match args.verbose {
        cli::Verbosity::Error => Level::Error,
        cli::Verbosity::Warning => Level::Warning,
        cli::Verbosity::Info => Level::Info,
        cli::Verbosity::Debug => Level::Debug,
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
            eprintln!("interrupted");
            return std::process::ExitCode::from(130);
        }
    };

    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            let peer_cancelled = matches!(
                e.downcast_ref::<hermod_core::orchestrate::OrchestrateError>(),
                Some(hermod_core::orchestrate::OrchestrateError::PeerCancelled)
            ) || e.to_string() == "peer cancelled the transfer";
            if peer_cancelled {
                // 对端取消：直接显示友好提示，不加 error: 前缀。
                eprintln!("peer cancelled the transfer");
            } else {
                verbosity::error(e.to_string());
            }
            std::process::ExitCode::from(1)
        }
    }
}

async fn run(
    args: cli::Cli,
    abort_slot: util::AbortSlot,
) -> Result<(), Box<dyn std::error::Error>> {
    let family = args.family();
    match args.cmd {
        cli::Command::Tx(a) => tx::run(a, args.network.options(false, family)?, abort_slot).await,
        cli::Command::Rx(a) => rx::run(a, args.network.options(true, family)?, abort_slot).await,
        cli::Command::Trust(a) => trust::run(a, family).await,
        cli::Command::Device(a) => device::run(a),
        // serve 不用全局 abort 槽：服务器常驻，Ctrl+C 由 main 的全局处理器结束进程。
        cli::Command::Serve(a) => serve::run(a, family).await,
    }
}
