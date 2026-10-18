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

    let result = tokio::select! {
        r = run(args) => r,
        _ = tokio::signal::ctrl_c() => {
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

async fn run(args: cli::Cli) -> Result<(), Box<dyn std::error::Error>> {
    match args.cmd {
        cli::Command::Tx(a) => tx::run(a).await,
        cli::Command::Rx(a) => rx::run(a).await,
        cli::Command::Trust(a) => trust::run(a).await,
        cli::Command::Serve => serve::run().await,
    }
}
