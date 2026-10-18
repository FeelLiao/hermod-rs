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
