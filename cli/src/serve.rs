//! `hermod serve`：信令 + NAT 辅助服务（Rust 版，对齐 Go `internal/cli/serve.go`）。
//!
//! 服务器只做信令中转、UDP 反射与 `/cert` 下发；文件数据走端到端 QUIC。
//! 与 Go server 同端口同协议，可互换部署；证书存配置目录 `server/` 子目录，
//! 首启自签 ECDSA P-256，到期前 14 天自动续期且密钥不变（SPKI 指纹不变）。

use crate::cli::ServeArgs;
use crate::{util, verbosity};
use hermod_core::options::IpFamily;
use hermod_core::server::{self, MemoryStore, Server, ServerLimits, cert};
use hermod_core::transport::network::bind_tcp;
use std::time::Duration;

/// 执行 `serve`。
pub async fn run(args: ServeArgs, family: IpFamily) -> Result<(), Box<dyn std::error::Error>> {
    let port = args.resolved_port()?;

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

    let listener = bind_tcp(port, family)?;
    let addr = listener.local_addr()?;
    // UDP 反射与 TCP 同端口（包括端口 0 动态分配）；失败降级为仅 TCP。
    let reflector = match server::reflector::start_with_family(addr, family) {
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

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
