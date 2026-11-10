//! CLI 仅解析交互参数；连接设置由核心库统一校验。
use clap::{Args, Parser, Subcommand};
use hermod_core::options::{Congestion, ConnectionSettings, LinkProfile, Mtu};
use hermod_core::server::{
    DEFAULT_MAX_BLOBS_PER_CHANNEL, DEFAULT_MAX_CHANNELS_PER_IP, DEFAULT_MAX_CPACE_FAILURES,
};

#[derive(Parser, Debug)]
#[command(
    name = "hermod",
    version,
    about = "Private LAN / peer-to-peer QUIC file and text transfer",
    after_help = "Migration: use --mtu instead of --wg-mtu/--quic-mtu; --congestion instead of --quic-cc. \
The WireGuard data plane and the --transport flag have been removed; transfers run over QUIC. \
tx/rx bind automatically (--listen was removed). \
`hermod serve` runs the signaling + NAT helper server."
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
    #[command(about = "Start the signaling and NAT helper service (drop-in for the Go server)")]
    Serve(ServeArgs),
}
#[derive(Args, Debug)]
pub struct NetworkArgs {
    #[arg(
        long,
        default_value = "auto",
        help = "MTU in bytes or auto; QUIC UDP payload cap"
    )]
    pub mtu: Mtu,
    #[arg(
        long,
        default_value = "auto",
        help = "auto (BBR) / cubic / bbr / newreno congestion control for QUIC"
    )]
    pub congestion: Congestion,
    #[arg(
        long,
        default_value = "auto",
        help = "auto / lan / compatible link profile: lan uses parallel \
                slice streams and faster congestion startup (best on WiFi LANs)"
    )]
    pub profile: LinkProfile,
}
impl NetworkArgs {
    pub fn options(&self, receiving: bool) -> Result<hermod_core::session::SessionOptions, String> {
        ConnectionSettings {
            transport: if receiving {
                "auto".into()
            } else {
                "quic".into()
            },
            mtu: self.mtu,
            congestion: self.congestion,
            profile: self.profile,
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

#[derive(Args, Debug)]
pub struct ServeArgs {
    #[arg(
        short,
        long,
        default_value = ":4376",
        help = "Bind address (host:port); HERMOD_LISTEN overrides the default"
    )]
    pub listen: String,
    #[arg(
        short = 'T',
        long,
        default_value_t = 600,
        help = "Idle timeout and channel TTL in seconds"
    )]
    pub ttl: u64,
    #[arg(
        long,
        default_value_t = 5.0,
        help = "Token bucket rate (requests/sec per IP prefix)"
    )]
    pub rate_limit: f64,
    #[arg(
        long,
        default_value_t = 15.0,
        help = "Token bucket burst capacity per IP prefix"
    )]
    pub rate_burst: f64,
    #[arg(
        long,
        default_value_t = DEFAULT_MAX_BLOBS_PER_CHANNEL,
        help = "Hard cap on relayed blobs per signaling channel"
    )]
    pub max_blobs_per_channel: usize,
    #[arg(
        long,
        default_value_t = DEFAULT_MAX_CPACE_FAILURES,
        help = "Max CPace handshake failures before a channel is invalidated"
    )]
    pub max_cpace_failures: usize,
    #[arg(
        long,
        default_value_t = DEFAULT_MAX_CHANNELS_PER_IP,
        help = "Max active channels per IP prefix (IPv4 /32, IPv6 /64)"
    )]
    pub max_channels_per_ip: usize,
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
        assert_eq!(
            tx.network.options(false).unwrap().transport,
            Some(hermod_core::transport::TransportKind::Quic)
        );
        let Command::Rx(rx) = Cli::try_parse_from(["hermod", "rx", "1-a-b-c"])
            .unwrap()
            .cmd
        else {
            panic!()
        };
        // 接收端 auto：transport None（跟随发送端），发送端恒为 QUIC
        assert_eq!(rx.network.options(true).unwrap().transport, None);
        assert_eq!(
            rx.network.options(false).unwrap().transport,
            Some(hermod_core::transport::TransportKind::Quic)
        );
        // 已经移除的 --transport 不再存在
        assert!(Cli::try_parse_from(["hermod", "tx", "file.bin", "--transport", "quic"]).is_err());
        assert!(Cli::try_parse_from(["hermod", "rx", "1-a-b-c", "--transport", "auto"]).is_err());
        for old in ["--wg-mtu", "--quic-mtu", "--quic-cc", "--listen"] {
            assert!(Cli::try_parse_from(["hermod", "tx", "file.bin", old, "1200"]).is_err());
        }
        // serve 回归为信令 server 子命令；tx 的第三个位置参数依旧不合法
        assert!(Cli::try_parse_from(["hermod", "tx", "file.bin", "serve"]).is_err());
    }

    /// serve 的默认旗标对齐 Go 参考实现（:4376 / TTL 600 / 5 rps burst 15 / 10 blob）。
    #[test]
    fn serve_defaults_match_go_reference() {
        let Command::Serve(serve) = Cli::try_parse_from(["hermod", "serve"]).unwrap().cmd else {
            panic!()
        };
        assert_eq!(serve.listen, ":4376");
        assert_eq!(serve.ttl, 600);
        assert_eq!(serve.rate_limit, 5.0);
        assert_eq!(serve.rate_burst, 15.0);
        assert_eq!(serve.max_blobs_per_channel, 10);
        assert_eq!(serve.max_cpace_failures, 3);
        assert_eq!(serve.max_channels_per_ip, 100);
    }
    #[test]
    fn invalid_combinations_fail_before_network_access() {
        let Command::Tx(tx) =
            Cli::try_parse_from(["hermod", "tx", "file.bin", "--congestion", "reno"])
                .unwrap()
                .cmd
        else {
            panic!()
        };
        // Reno 不属于 QUIC 支持集（历史上属于已移除的 WireGuard）
        assert!(
            tx.network.options(false).is_err(),
            "reno must be rejected for QUIC"
        );
    }
}
