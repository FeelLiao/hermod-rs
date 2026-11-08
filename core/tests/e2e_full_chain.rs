//! 验收层：只用 crate 的**公共 API**，经真实本地信令中继完成完整链路。
//!
//! 与 `src/` 内联单元测试互补——单元测试管实现细节与边界，这一层保证
//! 外部宿主（CLI / GUI 桥接）看到的端到端行为契约：
//! 传输码 → 会合 → CPace → 端点交换 → WG/QUIC 直连 → 隧道内 HTTP → SHA-256 裁决。

mod common;

use common::{start_relay, temp_dir};
use hermod_core::{
    bandwidth::BandwidthLimit,
    config::Config,
    crypto::wordlist,
    orchestrate::{self, OrchestrateError, ReceiveOptions},
    session::{self, SessionOptions},
    transfer::{Kind, TransferOffer, hash_bytes},
    transport::{AnyConnection, TransportConnection, TransportKind},
};
use rand_core::UnwrapErr;
use std::time::Duration;
use tokio::task::JoinHandle;

const SESSION_TIMEOUT: Duration = Duration::from_secs(90);
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(90);
const TEXT: &str = "验收层：一次完整链路。Hello, Hermod!\n";
const FILE_BYTES: usize = 4096;

fn quic_loopback() -> Option<std::net::SocketAddr> {
    // 本机 IPv6 loopback：不打洞、不过防火墙，保证测试在任何环境确定成功
    Some("[::1]:0".parse().unwrap())
}

/// 建立一对已配好的双方会话（发送端指定传输，接收端 auto 跟随），
/// 返回发送本端与下一个验收层终止条件。
async fn establish_pair(
    config: &Config,
    code: &str,
    transport: TransportKind,
    offer: Option<TransferOffer>,
) -> (
    hermod_core::session::TxSession,
    hermod_core::session::RxSession,
) {
    let mut sender_opts = SessionOptions {
        transport: Some(transport),
        udp_bind: quic_loopback(),
        ..Default::default()
    };
    sender_opts.offer = offer.clone();
    let receiver_opts = SessionOptions {
        transport: None,
        udp_bind: quic_loopback(),
        ..Default::default()
    };

    let sender = session::sender_session_with_ready(config, code, &sender_opts, || {});
    let receiver = session::receiver_session_with(config, code, &receiver_opts);
    // futures_util::future::join 才是真 future，才能进 tokio::time::timeout
    let (tx_result, rx_result) = tokio::time::timeout(
        SESSION_TIMEOUT,
        futures_util::future::join(sender, receiver),
    )
    .await
    .expect("session handshake deadline exceeded");
    let tx = tx_result.expect("sender session failed");
    let rx = rx_result.expect("receiver session failed");
    // 接收端必须自动跟随发送端的传输（当前均为 QUIC）
    assert!(matches!(tx.conn, AnyConnection::Quic(_)));
    assert!(matches!(rx.conn, AnyConnection::Quic(_)));
    if let Some(offer) = offer {
        assert_eq!(rx.offer.as_ref(), Some(&offer), "收端应看见加密预告");
    }
    (tx, rx)
}

/// 文本链路：传输码 → 会合 → 文本推送 → SHA-256 双向裁决。
async fn run_text_chain(transport: TransportKind) {
    let (config, relay) = start_relay().await;
    let mut rng = UnwrapErr(getrandom::SysRng);
    let (_, code) = wordlist::generate_transfer_code(&mut rng, 3);
    let offer = TransferOffer {
        kind: Kind::Text,
        name: "message.txt".into(),
        size: TEXT.len() as u64,
    };
    let (mut tx, mut rx) = establish_pair(&config, &code, transport, Some(offer)).await;

    let dir = temp_dir("text");
    let (sent, receipt) = tokio::time::timeout(
        TRANSFER_TIMEOUT,
        futures_util::future::join(
            async {
                orchestrate::send_text_with_limit(
                    &mut tx.conn,
                    tx.token,
                    TEXT,
                    None,
                    BandwidthLimit::UNLIMITED,
                )
                .await
                .unwrap()
            },
            async {
                orchestrate::receive_with_options(
                    &mut rx.conn,
                    &rx.token,
                    &dir,
                    ReceiveOptions {
                        expected: rx.offer.as_ref(),
                        ..Default::default()
                    },
                )
                .await
                .unwrap()
            },
        ),
    )
    .await
    .expect("transfer deadline exceeded");
    assert!(sent.verified);

    assert_eq!(receipt.kind, Kind::Text);
    assert_eq!(receipt.size as usize, TEXT.len());
    assert_eq!(receipt.sha256, hash_bytes(TEXT.as_bytes()));
    assert_eq!(std::fs::read(&receipt.path).unwrap(), TEXT.as_bytes());

    tx.conn.shutdown().await.unwrap();
    rx.conn.shutdown().await.unwrap();
    relay.abort();
    common::cleanup_dir(&dir);
}

/// 文件链路：预告（kind/name/size）先到 → 拉取落盘 → Receipt 完整可追。
async fn run_file_chain(transport: TransportKind) {
    let (config, relay) = start_relay().await;
    let mut rng = UnwrapErr(getrandom::SysRng);
    let mut data = vec![0u8; FILE_BYTES];
    use rand_core::Rng;
    rng.fill_bytes(&mut data);
    let dir = temp_dir("file");
    let source = dir.join("report.bin");
    std::fs::write(&source, &data).unwrap();

    let offer = TransferOffer {
        kind: Kind::File,
        name: "report.bin".into(),
        size: data.len() as u64,
    };
    let (_, code) = wordlist::generate_transfer_code(&mut rng, 3);
    let (mut tx, mut rx) = establish_pair(&config, &code, transport, Some(offer)).await;

    let dest_dir = dir.join("out");
    let (sent, receipt) = tokio::time::timeout(
        TRANSFER_TIMEOUT,
        futures_util::future::join(
            async {
                orchestrate::send_file_with_limit(
                    &mut tx.conn,
                    tx.token,
                    &source,
                    None,
                    BandwidthLimit::UNLIMITED,
                )
                .await
                .unwrap()
            },
            async {
                orchestrate::receive_with_options(
                    &mut rx.conn,
                    &rx.token,
                    &dest_dir,
                    ReceiveOptions {
                        expected: rx.offer.as_ref(),
                        ..Default::default()
                    },
                )
                .await
                .unwrap()
            },
        ),
    )
    .await
    .expect("transfer deadline exceeded");
    assert!(sent.verified);

    assert_eq!(receipt.path, dest_dir.join("report.bin"));
    assert_eq!(receipt.size as usize, FILE_BYTES);
    assert_eq!(receipt.kind, Kind::File);
    assert_eq!(receipt.sha256, hash_bytes(&data));
    assert_eq!(std::fs::read(&receipt.path).unwrap(), data);

    tx.conn.shutdown().await.unwrap();
    rx.conn.shutdown().await.unwrap();
    relay.abort();
    common::cleanup_dir(&dir);
}

/// 取消语义走公共 API：接收端主动 abort → 发送端得到干净的 PeerCancelled，
/// 而不是裸 io 错误（宿主把这种错误直接映射成"对方取消传输"文案）。
async fn peer_cancel_quic() {
    let (config, relay) = start_relay().await;
    let mut rng = UnwrapErr(getrandom::SysRng);
    let (_, code) = wordlist::generate_transfer_code(&mut rng, 3);
    let dir = temp_dir("cancel");

    let data = vec![0x5bu8; 64 * 1024 * 1024];
    let source = dir.join("cancel.bin");
    std::fs::write(&source, &data).unwrap();
    let offer = TransferOffer {
        kind: Kind::File,
        name: "cancel.bin".into(),
        size: data.len() as u64,
    };
    let (mut tx, mut rx) = establish_pair(&config, &code, TransportKind::Quic, Some(offer)).await;

    let started = std::sync::Arc::new(tokio::sync::Notify::new());
    let progress_started = started.clone();
    let abort = rx.conn.abort_handle();
    let kicker: JoinHandle<()> = tokio::spawn(async move {
        progress_started.notified().await;
        abort.abort();
    });

    let (sent, received) = tokio::time::timeout(
        TRANSFER_TIMEOUT,
        futures_util::future::join(
            async {
                orchestrate::send_file_with_limit(
                    &mut tx.conn,
                    tx.token,
                    &source,
                    Some(Box::new(move |n| {
                        if n > 0 {
                            started.notify_one();
                        }
                    })),
                    BandwidthLimit::UNLIMITED,
                )
                .await
            },
            async {
                orchestrate::receive_with_options(
                    &mut rx.conn,
                    &rx.token,
                    &dir.join("out"),
                    ReceiveOptions {
                        limit: BandwidthLimit::UNLIMITED,
                        expected: None,
                        ..Default::default()
                    },
                )
                .await
            },
        ),
    )
    .await
    .expect("cancel test deadline exceeded");

    // 取消语义：发起 abort 的一端看到的是自己本地连接的拆除（不是 PeerCancelled）；
    // 对端（tx）通过 QUIC cancel close code 得到干净的 PeerCancelled，而不是裸 io 错误。
    assert!(
        matches!(sent, Err(OrchestrateError::PeerCancelled)),
        "send result: {sent:?}"
    );
    assert!(
        received.is_err(),
        "aborting receiver must not complete successfully: {received:?}"
    );
    kicker.abort();
    relay.abort();
    common::cleanup_dir(&dir);
}

/// 传输驱动的深层调用路径在 Windows 默认小栈测试线程上可能栈溢出
/// （与 CLI 的 64 MiB 大栈线程同因，见 cli/src/main.rs）。验收层的整条
/// 链路都放到显式大栈线程执行。
#[allow(clippy::type_complexity)]
fn run_on_big_stack<T: Send + 'static>(work: Box<dyn FnOnce() -> T + Send>) -> T {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(work)
        .unwrap()
        .join()
        .unwrap()
}

#[test]
fn full_chain_text_over_quic() {
    run_on_big_stack(Box::new(|| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            run_text_chain(TransportKind::Quic).await;
        });
    }));
}

#[test]
fn full_chain_file_over_quic() {
    run_on_big_stack(Box::new(|| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            run_file_chain(TransportKind::Quic).await;
        });
    }));
}

#[test]
fn peer_cancel_quic_mid_transfer_over_relay() {
    run_on_big_stack(Box::new(|| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(peer_cancel_quic());
    }));
}
