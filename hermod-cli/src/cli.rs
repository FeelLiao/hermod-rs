//! 命令行参数（clap derive）。与 Go 版对齐，差异只允许附录 E：
//! 删除 `--verify/-v`（D12）、新增 `--wg-mtu`。

use clap::{Args, Parser, Subcommand};

/// Hermod：CPace + WireGuard 的点对点文件传输。
#[derive(Parser, Debug)]
#[command(name = "hermod", version, about = "Hermod — CPace + WireGuard 文件传输")]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Command,

    /// 日志级别
    #[arg(
        long,
        global = true,
        default_value = "none",
        value_parser = ["none", "error", "warning", "info", "debug"]
    )]
    pub verbose: String,

    /// 抑制状态输出（错误仍显示）
    #[arg(short, long, global = true)]
    pub quiet: bool,
}

/// 子命令。
#[derive(Subcommand, Debug)]
pub enum Command {
    /// 发送文件、文本或 stdin
    Tx(TxArgs),
    /// 用传输码接收
    Rx(RxArgs),
    /// 抓取并信任服务器证书（SPKI pinning）
    Trust(TrustArgs),
    /// 运行信令服务器（Rust 版未实现，D13）
    Serve,
}

/// `tx` 参数。
#[derive(Args, Debug)]
pub struct TxArgs {
    /// 文件路径、一段文本，或 `-` 表示 stdin
    pub input: String,

    /// 信令服务器 URL
    #[arg(short, long)]
    pub server: Option<String>,

    /// 传输码单词数
    #[arg(short, long, default_value_t = 3)]
    pub words: usize,

    /// UDP 绑定地址（IPv6）
    #[arg(short, long, default_value = "[::]:0")]
    pub listen: String,

    /// 内层 IP MTU
    #[arg(long, default_value_t = 1400)]
    pub wg_mtu: usize,
}

/// `rx` 参数。
#[derive(Args, Debug)]
pub struct RxArgs {
    /// 传输码（形如 `42-acid-acorn-acre`）
    pub code: String,

    /// 输出目录
    #[arg(short, long)]
    pub destination: Option<String>,

    /// 信令服务器 URL
    #[arg(short, long)]
    pub server: Option<String>,

    /// UDP 绑定地址（IPv6）
    #[arg(short, long, default_value = "[::]:0")]
    pub listen: String,

    /// 内层 IP MTU
    #[arg(long, default_value_t = 1400)]
    pub wg_mtu: usize,
}

/// `trust` 参数。
#[derive(Args, Debug)]
pub struct TrustArgs {
    /// 服务器 URL 或 host:port
    pub server: String,

    /// 已知 SPKI 指纹（hex）；给了就先比对再信任
    #[arg(long)]
    pub fingerprint: Option<String>,
}
