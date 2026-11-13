//! CLI 参数与调用前校验；调优不从配置文件读取。
use clap::{Args, Parser, Subcommand, ValueEnum};
use hermod_core::options::{Congestion, IpFamily, LinkProfile, Mtu, TransferTuning};
use hermod_core::server::{
    DEFAULT_MAX_BLOBS_PER_CHANNEL, DEFAULT_MAX_CHANNELS_PER_IP, DEFAULT_MAX_CPACE_FAILURES,
};
use hermod_core::session::SessionOptions;
use hermod_core::transport::TransportKind;
use std::time::Duration;

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CliCongestion {
    Auto,
    Bbr,
    Cubic,
    Newreno,
}
impl From<CliCongestion> for Congestion {
    fn from(value: CliCongestion) -> Self {
        match value {
            CliCongestion::Auto => Self::Auto,
            CliCongestion::Bbr => Self::Bbr,
            CliCongestion::Cubic => Self::Cubic,
            CliCongestion::Newreno => Self::Newreno,
        }
    }
}
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CliProfile {
    Auto,
    Lan,
    Wan,
}
impl From<CliProfile> for LinkProfile {
    fn from(value: CliProfile) -> Self {
        match value {
            CliProfile::Auto => Self::Auto,
            CliProfile::Lan => Self::Lan,
            CliProfile::Wan => Self::Wan,
        }
    }
}
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Verbosity {
    None,
    Error,
    Warning,
    Info,
    Debug,
}

#[derive(Parser, Debug)]
#[command(
    name = "hermod",
    version,
    about = "Private LAN / peer-to-peer QUIC file and text transfer"
)]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Command,
    #[arg(
        long,
        global = true,
        value_enum,
        ignore_case = true,
        default_value = "none",
        help = "Log verbosity"
    )]
    pub verbose: Verbosity,
    #[arg(
        short,
        long,
        global = true,
        help = "Suppress status output; errors are always shown"
    )]
    pub quiet: bool,
    #[arg(
        short = '4',
        long,
        global = true,
        conflicts_with = "ipv6",
        help = "Use IPv4 only"
    )]
    pub ipv4: bool,
    #[arg(
        short = '6',
        long,
        global = true,
        conflicts_with = "ipv4",
        help = "Use IPv6 only"
    )]
    pub ipv6: bool,
    #[command(flatten)]
    pub network: NetworkArgs,
}
#[derive(Subcommand, Debug)]
pub enum Command {
    #[command(about = "Send a file, text, or stdin")]
    Tx(TxArgs),
    #[command(about = "Receive using a transfer code")]
    Rx(RxArgs),
    #[command(about = "Fetch and pin a signaling server's SPKI fingerprint")]
    Trust(TrustArgs),
    #[command(about = "Start the signaling and NAT helper service")]
    Serve(ServeArgs),
}
#[derive(Args, Debug, Default)]
pub struct NetworkArgs {
    #[arg(short = 'm', long, global = true, value_parser = parse_mtu, help = "QUIC UDP payload cap: auto or 1200..1452 bytes [default: auto]")]
    pub mtu: Option<Mtu>,
    #[arg(
        short = 'c',
        long,
        global = true,
        value_enum,
        ignore_case = true,
        help = "Congestion control [default: bbr]"
    )]
    pub congestion: Option<CliCongestion>,
    #[arg(
        short = 'P',
        long,
        global = true,
        value_enum,
        ignore_case = true,
        help = "Link profile [default: auto]"
    )]
    pub profile: Option<CliProfile>,
    #[arg(long, global = true, value_parser = parse_streams, help = "Parallel streams: auto or 1..8 [default: auto]")]
    pub streams: Option<Streams>,
    #[arg(long, global = true, value_parser = parse_size_usize, help = "Chunk bytes, KiB or MiB; 256KiB..4MiB [default: 1MiB]")]
    pub chunk: Option<usize>,
    #[arg(long, global = true, value_parser = parse_size, help = "Per-stream window; 8MiB..256MiB [default: 64MiB]")]
    pub window: Option<u64>,
    #[arg(long, global = true, value_parser = parse_size_usize, help = "UDP socket buffer; 1MiB..64MiB [default: 16MiB]")]
    pub udp_buffer: Option<usize>,
    #[arg(long, global = true, value_parser = parse_size, help = "Slice size; 4MiB..64MiB [default: 4MiB]")]
    pub slice_size: Option<u64>,
    #[arg(
        long,
        global = true,
        help = "Prefetch queue depth: 4..64 [default: 16]"
    )]
    pub prefetch: Option<usize>,
    #[arg(
        long,
        global = true,
        help = "Keepalive seconds: 0 (off)..60 [default: 10]"
    )]
    pub keepalive: Option<u64>,
    #[arg(
        long,
        global = true,
        help = "Idle timeout seconds: 10..300, greater than keepalive [default: 30]"
    )]
    pub idle_timeout: Option<u64>,
}
#[derive(Debug, Clone, Copy)]
pub enum Streams {
    Auto,
    Count(usize),
}
impl NetworkArgs {
    fn explicit(&self) -> bool {
        self.mtu.is_some()
            || self.congestion.is_some()
            || self.profile.is_some()
            || self.streams.is_some()
            || self.chunk.is_some()
            || self.window.is_some()
            || self.udp_buffer.is_some()
            || self.slice_size.is_some()
            || self.prefetch.is_some()
            || self.keepalive.is_some()
            || self.idle_timeout.is_some()
    }
    pub fn options(&self, receiving: bool, family: IpFamily) -> Result<SessionOptions, String> {
        let defaults = TransferTuning::default();
        let tuning = TransferTuning {
            streams: match self.streams {
                Some(Streams::Count(n)) => Some(n),
                _ => None,
            },
            chunk: self.chunk.unwrap_or(defaults.chunk),
            window: self.window.unwrap_or(defaults.window),
            udp_buffer: self.udp_buffer.unwrap_or(defaults.udp_buffer),
            slice_size: self.slice_size.unwrap_or(defaults.slice_size),
            prefetch: self.prefetch.unwrap_or(defaults.prefetch),
            keepalive: self
                .keepalive
                .map(Duration::from_secs)
                .unwrap_or(defaults.keepalive),
            idle_timeout: self
                .idle_timeout
                .map(Duration::from_secs)
                .unwrap_or(defaults.idle_timeout),
        };
        tuning.validate()?;
        let options = SessionOptions {
            transport: if receiving {
                None
            } else {
                Some(TransportKind::Quic)
            },
            mtu: match self.mtu {
                Some(Mtu::Value(n)) => Some(usize::from(n)),
                _ => None,
            },
            congestion: self.congestion.unwrap_or(CliCongestion::Bbr).into(),
            profile: self.profile.unwrap_or(CliProfile::Auto).into(),
            family,
            tuning,
            ..Default::default()
        };
        options.validate_for(TransportKind::Quic)?;
        Ok(options)
    }
}
impl Cli {
    pub fn family(&self) -> IpFamily {
        if self.ipv4 {
            IpFamily::V4
        } else if self.ipv6 {
            IpFamily::V6
        } else {
            IpFamily::Auto
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        // clap 对分处子命令两侧的全局参数不检查互斥，传播完成后再兜底校验。
        if self.ipv4 && self.ipv6 {
            return Err("--ipv4 (-4) cannot be used with --ipv6 (-6)".into());
        }
        match &self.cmd {
            Command::Tx(_) | Command::Rx(_) => {
                self.network
                    .options(matches!(self.cmd, Command::Rx(_)), self.family())?;
            }
            Command::Serve(args) => {
                if self.network.explicit() {
                    return Err("data-plane options are only valid with tx/rx".into());
                }
                args.resolved_port()?;
            }
            Command::Trust(_) => {
                if self.network.explicit() {
                    return Err("data-plane options are only valid with tx/rx".into());
                }
            }
        }
        Ok(())
    }
}
fn parse_mtu(text: &str) -> Result<Mtu, String> {
    if text.eq_ignore_ascii_case("auto") {
        return Ok(Mtu::Auto);
    }
    let n: u16 = text.parse().map_err(|_| "MTU must be auto or 1200..1452")?;
    if !(1200..=1452).contains(&n) {
        return Err("MTU must be auto or 1200..1452".into());
    }
    Ok(Mtu::Value(n))
}
fn parse_streams(text: &str) -> Result<Streams, String> {
    if text.eq_ignore_ascii_case("auto") {
        return Ok(Streams::Auto);
    }
    let n = text
        .parse::<usize>()
        .map_err(|_| "streams must be auto or 1..8")?;
    if !(1..=8).contains(&n) {
        return Err("streams must be auto or 1..8".into());
    }
    Ok(Streams::Count(n))
}
fn parse_size(text: &str) -> Result<u64, String> {
    let (digits, multiplier) = if let Some(n) = text.strip_suffix("KiB") {
        (n, 1024)
    } else if let Some(n) = text.strip_suffix("MiB") {
        (n, 1024 * 1024)
    } else {
        (text, 1)
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err("size must be integer bytes or an integer followed by KiB/MiB".into());
    }
    digits
        .parse::<u64>()
        .ok()
        .and_then(|n| n.checked_mul(multiplier))
        .ok_or_else(|| "size is too large".into())
}
fn parse_size_usize(text: &str) -> Result<usize, String> {
    usize::try_from(parse_size(text)?).map_err(|_| "size is too large for this platform".into())
}
#[derive(Args, Debug)]
pub struct TxArgs {
    #[arg(help = "File path, text, or - for stdin")]
    pub input: String,
}
#[derive(Args, Debug)]
pub struct RxArgs {
    #[arg(help = "Transfer code")]
    pub code: String,
    #[arg(short = 'd', long, default_value = ".", help = "Output directory")]
    pub output: std::path::PathBuf,
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
        short = 'p',
        long,
        value_parser = parse_port,
        help = "Numeric listen port [default: HERMOD_PORT or 4376]"
    )]
    pub port: Option<u16>,
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
    #[arg(long, default_value_t = DEFAULT_MAX_BLOBS_PER_CHANNEL)]
    pub max_blobs_per_channel: usize,
    #[arg(long, default_value_t = DEFAULT_MAX_CPACE_FAILURES)]
    pub max_cpace_failures: usize,
    #[arg(long, default_value_t = DEFAULT_MAX_CHANNELS_PER_IP)]
    pub max_channels_per_ip: usize,
}
fn parse_port(text: &str) -> Result<u16, String> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err("port must be numeric (0..65535)".into());
    }
    text.parse()
        .map_err(|_| "port must be numeric (0..65535)".into())
}
impl ServeArgs {
    pub fn resolved_port(&self) -> Result<u16, String> {
        if let Some(port) = self.port {
            return Ok(port);
        }
        match std::env::var("HERMOD_PORT") {
            Ok(text) => {
                if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
                    return Err("HERMOD_PORT must be a numeric port (0..65535)".into());
                }
                text.parse()
                    .map_err(|_| "HERMOD_PORT must be a numeric port (0..65535)".into())
            }
            Err(std::env::VarError::NotPresent) => Ok(4376),
            Err(_) => Err("HERMOD_PORT must be a numeric port (0..65535)".into()),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_and_explicit_values() {
        let args = Cli::try_parse_from(["hermod", "tx", "text"]).unwrap();
        assert!(!args.network.explicit());
        let options = args.network.options(false, args.family()).unwrap();
        assert_eq!(options.congestion, Congestion::Bbr);
        assert_eq!(options.tuning.chunk, 1024 * 1024);
        let args = Cli::try_parse_from([
            "hermod", "-P", "WAN", "tx", "text", "-c", "CuBiC", "--chunk", "512KiB",
        ])
        .unwrap();
        let options = args.network.options(false, args.family()).unwrap();
        assert_eq!(options.profile, LinkProfile::Wan);
        assert_eq!(options.tuning.chunk, 512 * 1024);
    }
    #[test]
    fn explicit_tuning_and_family_reach_session_options() {
        let args = Cli::try_parse_from([
            "hermod",
            "-6",
            "--streams",
            "4",
            "--chunk",
            "256KiB",
            "--window",
            "8MiB",
            "rx",
            "1-acid-acorn-acre",
            "--udp-buffer",
            "1MiB",
            "--slice-size",
            "8MiB",
            "--prefetch",
            "4",
            "--keepalive",
            "0",
            "--idle-timeout",
            "10",
            "-d",
            "received",
        ])
        .unwrap();
        args.validate().unwrap();
        let opts = args.network.options(true, args.family()).unwrap();
        assert!(opts.transport.is_none());
        assert_eq!(opts.family, IpFamily::V6);
        assert_eq!(opts.tuning.streams, Some(4));
        assert_eq!(opts.tuning.chunk, 256 * 1024);
        assert_eq!(opts.tuning.window, 8 * 1024 * 1024);
        assert_eq!(opts.tuning.udp_buffer, 1024 * 1024);
        assert_eq!(opts.tuning.slice_size, 8 * 1024 * 1024);
        assert_eq!(opts.tuning.prefetch, 4);
        assert_eq!(opts.tuning.keepalive, Duration::ZERO);
        assert_eq!(opts.tuning.idle_timeout, Duration::from_secs(10));
        let Command::Rx(rx) = args.cmd else { panic!() };
        assert_eq!(rx.output, std::path::PathBuf::from("received"));
    }
    #[test]
    fn sizes_are_strict_and_checked() {
        for bad in [
            "1MB",
            "1.5MiB",
            "-1",
            "+1",
            "1 MiB",
            "18446744073709551615MiB",
        ] {
            assert!(parse_size(bad).is_err(), "{bad}");
        }
        assert_eq!(parse_size("1048576").unwrap(), parse_size("1MiB").unwrap());
    }
}
