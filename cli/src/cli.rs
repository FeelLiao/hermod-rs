//! CLI 仅解析交互参数；连接设置由核心库统一校验。
use clap::{Args, Parser, Subcommand};
use hermod_core::options::{Congestion, ConnectionSettings, Mtu};

#[derive(Parser, Debug)]
#[command(
    name = "hermod",
    version,
    about = "Private IPv4 LAN / IPv6 peer-to-peer file and text transfer",
    after_help = "Migration: use --mtu instead of --wg-mtu/--quic-mtu; --congestion instead of --quic-cc. Binding is automatic; --listen and the unimplemented serve command have been removed."
)]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Command,
    #[arg(long, global = true, default_value = "none", value_parser = ["none", "error", "warning", "info", "debug"], help = "Log verbosity")]
    pub verbose: String,
    #[arg(
        short,
        long,
        global = true,
        help = "Suppress status output; errors are always shown"
    )]
    pub quiet: bool,
}
#[derive(Subcommand, Debug)]
pub enum Command {
    #[command(about = "Send a file, text, or stdin")]
    Tx(TxArgs),
    #[command(about = "Receive using a transfer code")]
    Rx(RxArgs),
    #[command(about = "Fetch and pin a signaling server's SPKI fingerprint")]
    Trust(TrustArgs),
}
#[derive(Args, Debug)]
pub struct NetworkArgs {
    #[arg(
        long,
        default_value = "auto",
        help = "MTU in bytes or auto; WireGuard inner IP, QUIC UDP payload cap"
    )]
    pub mtu: Mtu,
    #[arg(
        long,
        default_value = "auto",
        help = "auto/cubic for either transport; reno for WireGuard; bbr/newreno for QUIC"
    )]
    pub congestion: Congestion,
    #[arg(
        long,
        default_value = "0",
        help = "Payload limit in Mbps (0 = unlimited)"
    )]
    pub limit_mbps: hermod_core::bandwidth::BandwidthLimit,
}
impl NetworkArgs {
    pub fn options(
        &self,
        transport: &str,
        receiving: bool,
    ) -> Result<hermod_core::session::SessionOptions, String> {
        ConnectionSettings {
            transport: transport.into(),
            mtu: self.mtu,
            congestion: self.congestion,
            limit_mbps: self.limit_mbps.to_string(),
        }
        .session_options(receiving)
    }
}
#[derive(Args, Debug)]
pub struct TxArgs {
    #[arg(help = "File path, text, or - for stdin")]
    pub input: String,
    #[arg(short, long, help = "Signaling server URL")]
    pub server: Option<String>,
    #[arg(short, long, default_value_t = 3, value_parser = clap::value_parser!(u8).range(1..=32), help = "Words in the transfer code")]
    pub words: u8,
    #[arg(long, default_value = "wireguard", value_parser = ["wireguard", "quic"], help = "Data-plane transport")]
    pub transport: String,
    #[command(flatten)]
    pub network: NetworkArgs,
}
#[derive(Args, Debug)]
pub struct RxArgs {
    #[arg(help = "Transfer code")]
    pub code: String,
    #[arg(short, long, help = "Output directory")]
    pub destination: Option<String>,
    #[arg(short, long, help = "Signaling server URL")]
    pub server: Option<String>,
    #[arg(long, default_value = "auto", value_parser = ["auto", "wireguard", "quic"], help = "auto follows the sender")]
    pub transport: String,
    #[command(flatten)]
    pub network: NetworkArgs,
}
#[derive(Args, Debug)]
pub struct TrustArgs {
    #[arg(help = "Server URL or host:port")]
    pub server: String,
    #[arg(long, help = "Expected SPKI fingerprint (SHA-256 hex)")]
    pub fingerprint: Option<String>,
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_and_removed_options() {
        let Command::Tx(tx) = Cli::try_parse_from(["hermod", "tx", "file.bin"])
            .unwrap()
            .cmd
        else {
            panic!()
        };
        assert_eq!(tx.transport, "wireguard");
        assert_eq!(tx.network.options(&tx.transport, false).unwrap().mtu, None);
        let Command::Rx(rx) = Cli::try_parse_from(["hermod", "rx", "1-a-b-c"])
            .unwrap()
            .cmd
        else {
            panic!()
        };
        assert_eq!(rx.transport, "auto");
        for old in ["--wg-mtu", "--quic-mtu", "--quic-cc", "--listen"] {
            assert!(Cli::try_parse_from(["hermod", "tx", "file.bin", old, "1200"]).is_err());
        }
        assert!(Cli::try_parse_from(["hermod", "serve"]).is_err());
    }
    #[test]
    fn invalid_combinations_fail_before_network_access() {
        let Command::Tx(tx) =
            Cli::try_parse_from(["hermod", "tx", "file.bin", "--congestion", "bbr"])
                .unwrap()
                .cmd
        else {
            panic!()
        };
        assert!(tx.network.options(&tx.transport, false).is_err());
    }
}
