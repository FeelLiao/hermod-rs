//! 命令行参数（clap derive）。与 Go 版对齐，差异只允许附录 E：
//! 删除 `--verify/-v`（D12）、新增 `--wg-mtu`。

use clap::{Args, Parser, Subcommand};

/// Hermod：CPace + WireGuard 的点对点文件传输。
#[derive(Parser, Debug)]
#[command(name = "hermod", version, about = "Hermod - CPace + WireGuard file transfer")]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Command,

    /// Log verbosity
    #[arg(
        long,
        global = true,
        default_value = "none",
        value_parser = ["none", "error", "warning", "info", "debug"]
    )]
    pub verbose: String,

    /// Suppress status output (errors are always shown)
    #[arg(short, long, global = true)]
    pub quiet: bool,
}

/// 子命令。
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Send a file, text, or stdin
    Tx(TxArgs),
    /// Receive using a transfer code
    Rx(RxArgs),
    /// Fetch and pin a server certificate (SPKI)
    Trust(TrustArgs),
    /// Run the signaling server (Rust: not implemented, D13)
    Serve,
}

/// `tx` 参数。
#[derive(Args, Debug)]
pub struct TxArgs {
    /// File path, text, or `-` for stdin
    pub input: String,

    /// Signaling server URL
    #[arg(short, long)]
    pub server: Option<String>,

    /// Number of words in the transfer code
    #[arg(short, long, default_value_t = 3)]
    pub words: usize,

    /// UDP bind address (IPv6)
    #[arg(short, long, default_value = "[::]:0")]
    pub listen: String,

    /// Inner IP MTU
    #[arg(long, default_value_t = 1400)]
    pub wg_mtu: usize,
}

/// `rx` 参数。
#[derive(Args, Debug)]
pub struct RxArgs {
    /// Transfer code (e.g. `42-acid-acorn-acre`)
    pub code: String,

    /// Output directory
    #[arg(short, long)]
    pub destination: Option<String>,

    /// Signaling server URL
    #[arg(short, long)]
    pub server: Option<String>,

    /// UDP bind address (IPv6)
    #[arg(short, long, default_value = "[::]:0")]
    pub listen: String,

    /// Inner IP MTU
    #[arg(long, default_value_t = 1400)]
    pub wg_mtu: usize,
}

/// `trust` 参数。
#[derive(Args, Debug)]
pub struct TrustArgs {
    /// Server URL or host:port
    pub server: String,

    /// Known SPKI fingerprint (hex); verify before trusting
    #[arg(long)]
    pub fingerprint: Option<String>,
}
