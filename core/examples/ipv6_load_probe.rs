//! 隔离网络实验用：本机 IPv6 QUIC 文件流（协议 v2 直连流帧）。
//! 不使用信令服务器，不测公网可达性；固定凭证仅限本地实验。

use hermod_core::crypto::cpace::Role;
use hermod_core::orchestrate::{self, ServeOptions};
use hermod_core::transfer::{Kind, TransferOffer};
use hermod_core::transport::{Transport, TransportConnection, quic::transport::QuicTransport};
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut tx = QuicTransport::new();
    let mut rx = QuicTransport::new();
    let bind = Some("[::1]:0".parse()?);
    tx.set_udp_bind(bind);
    rx.set_udp_bind(bind);
    tx.set_credential_fingerprint([7; 32]);
    rx.set_credential_fingerprint([7; 32]);
    let tx_endpoint = tx.prepare().await?;
    let rx_endpoint = rx.prepare().await?;
    let mut tx = tx.establish(Role::Sender, &[0; 32], &rx_endpoint).await?;
    let mut rx = rx.establish(Role::Receiver, &[0; 32], &tx_endpoint).await?;
    let size = 8 * 1024 * 1024;
    let offer = TransferOffer {
        kind: Kind::File,
        name: "probe.bin".into(),
        size,
    };
    let options = ServeOptions {
        kind: Kind::File,
        name: offer.name.clone(),
        size,
        body: Box::new(std::io::Cursor::new(vec![0x5a; size as usize])),
        progress: None,
    };
    let mut sink = tokio::io::sink();
    let started = Instant::now();
    let (sent, received) = tokio::time::timeout(Duration::from_secs(120), async {
        tokio::try_join!(
            orchestrate::serve_file(&mut tx, options),
            orchestrate::fetch_with_options(
                rx.dial().await.map_err(|e| {
                    use hermod_core::orchestrate::OrchestrateError;
                    OrchestrateError::Transport(e.to_string())
                })?,
                &offer,
                &mut sink,
                None,
            )
        )
    })
    .await??;
    let (sha, _) = received;
    let verified = sent.verified && sha == sent.sha256;
    assert!(verified);
    tx.shutdown().await?;
    rx.shutdown().await?;
    println!(
        "elapsed_seconds={:.3} payload_mbps={:.3} sha256={}",
        started.elapsed().as_secs_f64(),
        size as f64 * 8.0 / started.elapsed().as_secs_f64() / 1_000_000.0,
        sent.sha256
    );
    Ok(())
}
