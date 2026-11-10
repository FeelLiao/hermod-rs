//! `hermod serve`：信令 + NAT 辅助服务（Rust 版，对齐 Go `internal/cli/serve.go`）。
//!
//! 服务器只做信令中转、UDP 反射与 `/cert` 下发；文件数据走端到端 QUIC。
//! 与 Go server 同端口同协议，可互换部署；证书存配置目录 `server/` 子目录，
//! 首启自签 ECDSA P-256，到期前 14 天自动续期且密钥不变（SPKI 指纹不变）。

use crate::cli::ServeArgs;
use crate::{util, verbosity};
use hermod_core::server::{self, MemoryStore, Server, ServerLimits, cert};
use std::net::SocketAddr;
use std::time::Duration;

/// 执行 `serve`。
pub async fn run(args: ServeArgs) -> Result<(), Box<dyn std::error::Error>> {
    // HERMOD_LISTEN 只在未显式传 --listen 时生效（对应 Go envOrDefault 的优先级）。
    let listen = if args.listen == ":4376" {
        std::env::var("HERMOD_LISTEN").unwrap_or(args.listen.clone())
    } else {
        args.listen.clone()
    };
    let addr = parse_listen(&listen)?;

    // 证书：配置目录 server/ 子目录（首启生成；到期前自动续期，密钥不变）。
    let cert_dir = hermod_core::config::dir().join("server");
    let cert = cert::ensure(&cert_dir, cert::CERT_AUTO_RENEW_THRESHOLD).map_err(util::boxed)?;
    verbosity::info(format!(
        "server certificate ready; spki sha256: {}",
        hex(&cert.spki_fingerprint),
    ));

    let acceptor = server::tls_acceptor(&cert).map_err(util::boxed)?;
    let store = std::sync::Arc::new(MemoryStore::new(args.max_channels_per_ip));
    let server = Server::new(
        ServerLimits {
            ttl: Duration::from_secs(args.ttl),
            max_blobs_per_channel: args.max_blobs_per_channel,
            max_cpace_failures: args.max_cpace_failures,
        },
        store,
        args.rate_limit,
        args.rate_burst,
        cert.cert_pem.clone(),
    );

    let listener = tokio::net::TcpListener::bind(addr).await?;
    // UDP 反射与 TCP 同端口（CGNAT 外部地址发现）；失败降级为仅 TCP，与 Go 一致。
    let reflector = match server::reflector::start(addr) {
        Ok(handle) => Some(handle),
        Err(error) => {
            crate::verbosity::info(format!(
                "UDP reflection unavailable ({error}) — CGNAT peers may not connect",
            ));
            None
        }
    };

    verbosity::status(format!("Listening on {addr}"));
    verbosity::status(format!(
        "Server public key fingerprint: {}",
        hex(&cert.spki_fingerprint)
    ));
    verbosity::info(format!(
        "channel_ttl={}s rate_limit={}/s rate_burst={} max_blobs_per_channel={} max_cpace_failures={} max_channels_per_ip={}",
        args.ttl,
        args.rate_limit,
        args.rate_burst,
        args.max_blobs_per_channel,
        args.max_cpace_failures,
        args.max_channels_per_ip,
    ));

    // Ctrl+C 由 main 的全局处理器接管（打印 interrupted 并以 130 退出）。
    server.run(listener, acceptor, reflector).await?;
    Ok(())
}

/// 解析监听地址：":4376" 补全为双栈 `[::]:4376`（对应 Go `":4376"` 的语义）。
fn parse_listen(text: &str) -> Result<SocketAddr, String> {
    let normalized = match text.strip_prefix(':') {
        Some(port) => format!("[::]:{port}"),
        None => text.to_string(),
    };
    normalized
        .parse()
        .map_err(|_| format!("invalid listen address: {text}"))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colon_listen_binds_dual_stack() {
        let addr = parse_listen(":4376").unwrap();
        assert_eq!(addr.port(), 4376);
        assert!(addr.is_ipv6());
        // 显式地址原样解析
        let explicit = parse_listen("0.0.0.0:4376").unwrap();
        assert!(explicit.is_ipv4());
        assert!(parse_listen("not-an-addr").is_err());
    }
}
