//! QUIC 公开 API 集成测试：端点、凭证、证书钉扎和手动吞吐验证。

use hermod_core::{
    crypto::cpace::Role,
    transport::{
        Transport, TransportConnection,
        quic::{
            bundle::{QUIC_ENDPOINT_VERSION, QuicEndpoint},
            transport::{QuicError, QuicTransport},
        },
    },
};
use std::{
    net::{Ipv6Addr, SocketAddrV6},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn loopback_transport() -> QuicTransport {
    let mut transport = QuicTransport::new();
    transport.set_udp_bind(Some(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 0, 0, 0).into()));
    transport
}

/// prepare 产出的 endpoint 是 QUIC 类型，带证书指纹和可用端口。
#[tokio::test]
async fn prepare_produces_quic_endpoint() {
    let mut t = loopback_transport();
    t.set_credential_fingerprint([0x42; 32]);
    let bytes = t.prepare().await.unwrap();
    let b = QuicEndpoint::decode(&bytes).unwrap();
    assert_eq!(b.version, QUIC_ENDPOINT_VERSION);
    assert_eq!(b.credential_fingerprint, [0x42; 32]);
    assert_ne!(b.cert_fingerprint, [0u8; 32]);
    assert_ne!(b.udp_port, 0);
    assert!(b.ipv6_candidates.contains(&Ipv6Addr::LOCALHOST));
}

/// 凭证指纹不一致必须在握手前拒绝。
#[tokio::test]
async fn credential_mismatch_is_rejected() {
    let mut tx = loopback_transport();
    let mut rx = loopback_transport();
    tx.set_credential_fingerprint([0x01; 32]);
    rx.set_credential_fingerprint([0x02; 32]);
    let _tx_ep = tx.prepare().await.unwrap();
    let rx_ep = rx.prepare().await.unwrap();
    let psk = hermod_core::crypto::kdf::derive_wg_psk(&[0x88; 32], 3);
    let err = tx.establish(Role::Sender, &psk, &rx_ep).await.err();
    assert!(matches!(err, Some(QuicError::CredentialMismatch)));
}

/// 错误的对端证书指纹必须让握手失败（钉扎生效）。
#[tokio::test]
async fn wrong_certificate_fingerprint_fails_handshake() {
    let mut tx = loopback_transport();
    let mut rx = loopback_transport();
    tx.set_credential_fingerprint([0x33; 32]);
    rx.set_credential_fingerprint([0x33; 32]);
    let tx_ep = tx.prepare().await.unwrap();
    let mut rx_ep = QuicEndpoint::decode(&rx.prepare().await.unwrap()).unwrap();
    // 客户端只试 ::1，避免在测试里等不可达候选的超时。
    rx_ep.ipv6_candidates = vec![Ipv6Addr::LOCALHOST];
    // 把 rx（QUIC server）公布的证书指纹改坏；tx（client）必须拒绝它。
    rx_ep.cert_fingerprint = [0x00; 32];
    let rx_ep = rx_ep.encode().unwrap();
    let psk = hermod_core::crypto::kdf::derive_wg_psk(&[0x99; 32], 4);

    let mut tx_conn = tx.establish(Role::Sender, &psk, &rx_ep).await.unwrap();
    let _rx_conn = rx.establish(Role::Receiver, &psk, &tx_ep).await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(10), tx_conn.listen()).await;
    assert!(
        matches!(result, Ok(Err(_))),
        "握手必须因为证书指纹不匹配而失败"
    );
}

/// 手动吞吐基准：`cargo test -p hermod-core --release -- --ignored quic_throughput`。
#[tokio::test]
#[ignore = "throughput benchmark; run manually in release mode"]
async fn quic_throughput() {
    let mut tx = loopback_transport();
    let mut rx = loopback_transport();
    tx.set_credential_fingerprint([0x44; 32]);
    rx.set_credential_fingerprint([0x44; 32]);
    let tx_ep = tx.prepare().await.unwrap();
    let rx_ep = rx.prepare().await.unwrap();
    let psk = hermod_core::crypto::kdf::derive_wg_psk(&[0xaa; 32], 1);
    let mut tx_conn = tx.establish(Role::Sender, &psk, &rx_ep).await.unwrap();
    let rx_conn = rx.establish(Role::Receiver, &psk, &tx_ep).await.unwrap();

    const TOTAL: usize = 256 * 1024 * 1024;
    let data = vec![0xa5_u8; 1024 * 1024];
    let start = std::time::Instant::now();
    let client_task = tokio::spawn(async move {
        let mut client_conn = rx_conn;
        let mut client = client_conn.dial().await.unwrap();
        for _ in 0..(TOTAL / data.len()) {
            client.write_all(&data).await.unwrap();
        }
        client.shutdown().await.unwrap();
        client_conn
    });

    let mut server = tx_conn.listen().await.unwrap();
    let mut buf = vec![0u8; 1024 * 1024];
    let mut total = 0usize;
    loop {
        let n = server.read(&mut buf).await.unwrap();
        if n == 0 {
            break;
        }
        total += n;
    }
    let mut rx_conn = client_task.await.unwrap();
    let secs = start.elapsed().as_secs_f64();
    println!(
        "QUIC throughput: {:.1} MiB/s ({total} bytes in {secs:.3}s)",
        total as f64 / 1_048_576.0 / secs
    );
    assert_eq!(total, TOTAL);
    tx_conn.shutdown().await.unwrap();
    rx_conn.shutdown().await.unwrap();
}
