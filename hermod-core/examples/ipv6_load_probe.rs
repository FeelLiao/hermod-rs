//! 隔离网络实验用：本机 IPv6 QUIC 文件流，可配合 tc/ping 对照不限速和限速。
//! 不使用信令服务器，不测公网可达性；固定凭证仅限本地实验。

use hermod_core::transport::{
    Transport, TransportConnection, http::server::ServeOptions, quic::transport::QuicTransport,
};
use hermod_core::{bandwidth::BandwidthLimit, crypto::cpace::Role, orchestrate, transfer::Kind};
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let limit: BandwidthLimit = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "0".into())
        .parse()?;
    let mut tx = QuicTransport::new();
    let mut rx = QuicTransport::new();
    let bind = Some("[::1]:0".parse()?);
    tx.set_udp_bind(bind);
    rx.set_udp_bind(bind);
    tx.set_bandwidth_limit(limit);
    tx.set_credential_fingerprint([7; 32]);
    rx.set_credential_fingerprint([7; 32]);
    let tx_endpoint = tx.prepare().await?;
    let rx_endpoint = rx.prepare().await?;
    let mut tx = tx.establish(Role::Sender, &[0; 32], &rx_endpoint).await?;
    let mut rx = rx.establish(Role::Receiver, &[0; 32], &tx_endpoint).await?;
    let size = 8 * 1024 * 1024;
    let options = ServeOptions {
        token: [7; 16],
        kind: Kind::File,
        name: "probe.bin".into(),
        size,
        body: Box::new(std::io::Cursor::new(vec![0x5a; size as usize])),
        progress: None,
    };
    let mut sink = tokio::io::sink();
    let started = Instant::now();
    let (sent, received) = tokio::time::timeout(Duration::from_secs(120), async {
        tokio::try_join!(
            orchestrate::serve_file_with_limit(&mut tx, options, limit),
            orchestrate::fetch_file(&mut rx, &[7; 16], &mut sink, None)
        )
    })
    .await??;
    assert!(sent.verified && received.verified);
    tx.shutdown().await?;
    rx.shutdown().await?;
    println!(
        "limit_mbps={} elapsed_seconds={:.3} payload_mbps={:.3} sha256={}",
        limit,
        started.elapsed().as_secs_f64(),
        size as f64 * 8.0 / started.elapsed().as_secs_f64() / 1_000_000.0,
        sent.sha256
    );
    Ok(())
}
