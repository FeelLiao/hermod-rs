//! 通过公开 API 验证真实 QUIC 的载荷传输与两端调优不一致时的互通。

use hermod_core::{
    crypto::{cpace::Role, kdf::derive_wg_psk},
    options::{IpFamily, LinkProfile, TransferTuning as ConnectionTuning},
    orchestrate::{ReceiveOptions, receive, send_file, send_text},
    transfer::{Kind, TransferOffer, hash_bytes},
    transport::{QuicConnection, QuicTransport, Transport, TransportConnection},
};
use std::{
    net::{Ipv6Addr, SocketAddrV6},
    time::Duration,
};

/// 建一对已建立、可 listen/dial 的连接（QUIC 回环）。
async fn connected_pair() -> (QuicConnection, QuicConnection) {
    let mut tx = QuicTransport::new();
    let mut rx = QuicTransport::new();
    let bind: std::net::SocketAddr = SocketAddrV6::new(Ipv6Addr::LOCALHOST, 0, 0, 0).into();
    tx.set_udp_bind(Some(bind));
    rx.set_udp_bind(Some(bind));
    tx.set_credential_fingerprint([0x66; 32]);
    rx.set_credential_fingerprint([0x66; 32]);
    let tx_ep = tx.prepare().await.unwrap();
    let rx_ep = rx.prepare().await.unwrap();
    let psk = derive_wg_psk(&[0x44u8; 32], 7);
    let tx_conn = tx.establish(Role::Sender, &psk, &rx_ep).await.unwrap();
    let rx_conn = rx.establish(Role::Receiver, &psk, &tx_ep).await.unwrap();
    (tx_conn, rx_conn)
}

/// 终极 e2e：两个 QuicTransport + 单流帧，完整搬运一段文本并裁决。
#[tokio::test]
async fn end_to_end_file_transfer() {
    let (mut tx_conn, mut rx_conn) = connected_pair().await;
    let data = "hermod end-to-end file transfer!".to_string();
    let offer = TransferOffer {
        kind: Kind::File,
        name: "hello.txt".into(),
        size: data.len() as u64,
    };

    let dir = std::env::temp_dir().join(format!("hermod-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let dest_dir = dir.join("out");

    let (serve_out, receipt) = tokio::join!(
        async { send_text(&mut tx_conn, &data, None).await.unwrap() },
        async {
            receive(
                &mut rx_conn,
                &dest_dir,
                ReceiveOptions {
                    offer: Some(&offer),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
        },
    );

    assert!(serve_out.verified);
    assert_eq!(std::fs::read(&receipt.path).unwrap(), data.as_bytes());
    assert_eq!(receipt.sha256, hash_bytes(data.as_bytes()));
    assert_eq!(receipt.path.file_name().unwrap(), "hello.txt");
    let _ = std::fs::remove_dir_all(&dir);
    TransportConnection::shutdown(&mut tx_conn).await.unwrap();
    TransportConnection::shutdown(&mut rx_conn).await.unwrap();
}

/// 多流切片 e2e：真实文件源 + 局域网档，17 MiB 走多流路径，
/// 校验字节级一致与哈希一致。
#[tokio::test]
async fn end_to_end_multistream_transfer() {
    let (mut tx_conn, mut rx_conn) = connected_pair().await;
    // 17 MiB 确定性伪随机载荷（LCG，不引依赖）
    let mut data = Vec::with_capacity(17 * 1024 * 1024);
    let mut state = 0x1234_5678_u32;
    for _ in 0..data.capacity() {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        data.push((state >> 24) as u8);
    }
    let dir = std::env::temp_dir().join(format!("hermod-e2e-multi-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("payload.bin");
    std::fs::write(&src, &data).unwrap();
    let offer = TransferOffer {
        kind: Kind::File,
        name: "payload.bin".into(),
        size: data.len() as u64,
    };
    let dest_dir = dir.join("out");

    let (serve_out, receipt) = tokio::join!(
        async { send_file(&mut tx_conn, &src, None).await.unwrap() },
        async {
            receive(
                &mut rx_conn,
                &dest_dir,
                ReceiveOptions {
                    offer: Some(&offer),
                    profile: LinkProfile::Lan,
                    peer_multi_stream: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap()
        },
    );

    assert!(serve_out.verified);
    assert_eq!(receipt.sha256, hash_bytes(&data));
    assert_eq!(std::fs::read(&receipt.path).unwrap(), data);
    let _ = std::fs::remove_dir_all(&dir);
    TransportConnection::shutdown(&mut tx_conn).await.unwrap();
    TransportConnection::shutdown(&mut rx_conn).await.unwrap();
}

#[tokio::test]
async fn quic_distinct_chunks_and_slice_windows_interoperate() {
    for (streams, slice_size) in [(2, 4 * 1024 * 1024 + 7), (8, 5 * 1024 * 1024 + 13)] {
        let mut tx = QuicTransport::new();
        let mut rx = QuicTransport::new();
        tx.set_transfer_options(
            IpFamily::V4,
            ConnectionTuning {
                streams: Some(1),
                chunk: 256 * 1024,
                prefetch: 4,
                ..Default::default()
            },
        );
        rx.set_transfer_options(
            IpFamily::V4,
            ConnectionTuning {
                streams: Some(streams),
                chunk: 1024 * 1024 + 53,
                slice_size,
                prefetch: 4,
                ..Default::default()
            },
        );
        tx.set_udp_bind(Some("127.0.0.1:0".parse().unwrap()));
        rx.set_udp_bind(Some("127.0.0.1:0".parse().unwrap()));
        tx.set_credential_fingerprint([0x71; 32]);
        rx.set_credential_fingerprint([0x71; 32]);
        let tx_ep = tx.prepare().await.unwrap();
        let rx_ep = rx.prepare().await.unwrap();
        let psk = derive_wg_psk(&[0x72; 32], 7);
        let mut tx = tx.establish(Role::Sender, &psk, &rx_ep).await.unwrap();
        let mut rx = rx.establish(Role::Receiver, &psk, &tx_ep).await.unwrap();
        let dir = std::env::temp_dir().join(format!(
            "hermod-quic-tuning-{}-{streams}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.bin");
        let data: Vec<u8> = (0..48 * 1024 * 1024 + 97)
            .map(|i| (i % 251) as u8)
            .collect();
        std::fs::write(&source, &data).unwrap();
        let offer = TransferOffer {
            kind: Kind::File,
            name: "received.bin".into(),
            size: data.len() as u64,
        };
        let out = dir.join("out");
        let (sent, receipt) = tokio::time::timeout(Duration::from_secs(45), async {
            tokio::join!(
                send_file(&mut tx, &source, None),
                receive(
                    &mut rx,
                    &out,
                    ReceiveOptions {
                        offer: Some(&offer),
                        profile: LinkProfile::Wan,
                        peer_multi_stream: true,
                        ..Default::default()
                    }
                )
            )
        })
        .await
        .expect("QUIC stream credit must replenish without deadlock");
        let receipt = receipt.unwrap();
        assert!(sent.unwrap().verified);
        assert_eq!(receipt.sha256, hash_bytes(&data));
        assert_eq!(std::fs::read(&receipt.path).unwrap(), data);
        tx.shutdown().await.unwrap();
        rx.shutdown().await.unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }
}
