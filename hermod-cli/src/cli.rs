//! 命令行参数（clap derive）。与 Go 版对齐，差异只允许附录 E：
//! 删除 `--verify/-v`（D12）、新增 `--wg-mtu`、`--transport`、
//! `--quic-cc` 与 `--quic-mtu`。

use clap::{Args, Parser, Subcommand};

/// Hermod：CPace + WireGuard/QUIC 的点对点文件传输。
#[derive(Parser, Debug)]
#[command(
    name = "hermod",
    version,
    about = "Hermod - CPace + WireGuard/QUIC file transfer"
)]
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

/// tx 的传输参数：发送端必须选一个实现，默认 QUIC。
#[derive(Args, Debug)]
pub struct TxTransportArgs {
    /// Data-plane transport (default: quic)
    #[arg(long, default_value = "quic", value_parser = ["wireguard", "quic"])]
    pub transport: String,

    /// QUIC congestion control (ignored for WireGuard)
    #[arg(long, default_value = "bbr", value_parser = ["bbr", "cubic", "newreno"])]
    pub quic_cc: String,

    /// QUIC initial UDP payload in bytes, 1200..=1452 (ignored for WireGuard)
    #[arg(long, default_value_t = 1200)]
    pub quic_mtu: u16,
}

/// rx 的传输参数：默认 auto，跟随发送端的端点头。
#[derive(Args, Debug)]
pub struct RxTransportArgs {
    /// Data-plane transport; auto follows the sender's endpoint header
    #[arg(long, default_value = "auto", value_parser = ["auto", "wireguard", "quic"])]
    pub transport: String,

    /// QUIC congestion control (ignored for WireGuard)
    #[arg(long, default_value = "bbr", value_parser = ["bbr", "cubic", "newreno"])]
    pub quic_cc: String,

    /// QUIC initial UDP payload in bytes, 1200..=1452 (ignored for WireGuard)
    #[arg(long, default_value_t = 1200)]
    pub quic_mtu: u16,
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

    /// WireGuard inner IP MTU (ignored for QUIC)
    #[arg(long, default_value_t = 1400)]
    pub wg_mtu: usize,

    #[command(flatten)]
    pub transport_opts: TxTransportArgs,
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

    /// WireGuard inner IP MTU (ignored for QUIC)
    #[arg(long, default_value_t = 1400)]
    pub wg_mtu: usize,

    #[command(flatten)]
    pub transport_opts: RxTransportArgs,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tx_transport_defaults_to_quic() {
        let cli = Cli::try_parse_from(["hermod", "tx", "file.bin"]).unwrap();
        match cli.cmd {
            Command::Tx(args) => assert_eq!(args.transport_opts.transport, "quic"),
            _ => panic!("expected tx"),
        }
    }

    #[test]
    fn rx_transport_defaults_to_auto() {
        let cli = Cli::try_parse_from(["hermod", "rx", "1-a-b-c"]).unwrap();
        match cli.cmd {
            Command::Rx(args) => assert_eq!(args.transport_opts.transport, "auto"),
            _ => panic!("expected rx"),
        }
    }
}
