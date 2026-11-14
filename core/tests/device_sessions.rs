//! 真实 Rust 信令服务上的显式配对与设备 ID 双门传输。
use hermod_core::{
    config::Config,
    contacts::Contacts,
    identity::Identity,
    options::IpFamily,
    orchestrate::{self, ReceiveOptions},
    server::{MemoryStore, Server, ServerLimits, cert},
    session::{self, SessionOptions},
    transfer::{Kind, TransferOffer, hash_bytes},
    transport::{TransportConnection, network},
};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

#[path = "support/storage_permissions.rs"]
mod storage_permissions;

struct Fixture {
    dir: PathBuf,
    config: Config,
    task: tokio::task::JoinHandle<()>,
}
impl Fixture {
    async fn new() -> Self {
        storage_permissions::install();
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let dir = std::env::temp_dir().join(format!(
            "hermod-device-session-{}",
            hermod_core::transfer::hex(&random)
        ));
        let certificate =
            cert::ensure(&dir.join("server"), cert::CERT_AUTO_RENEW_THRESHOLD).unwrap();
        let listener = network::bind_tcp(0, IpFamily::V4).unwrap();
        let address = listener.local_addr().unwrap();
        let url = format!("wss://127.0.0.1:{}", address.port());
        let mut config = Config::default();
        config
            .replace_trust(
                &url,
                &hermod_core::transfer::hex(&certificate.spki_fingerprint),
            )
            .unwrap();
        let server = Server::new(
            ServerLimits {
                ttl: Duration::from_secs(60),
                max_blobs_per_channel: hermod_core::server::DEFAULT_MAX_BLOBS_PER_CHANNEL,
                max_cpace_failures: 3,
            },
            std::sync::Arc::new(MemoryStore::new(100)),
            1000.0,
            3000.0,
            certificate.cert_pem.clone(),
        );
        let acceptor = hermod_core::server::tls_acceptor(&certificate).unwrap();
        let task = tokio::spawn(async move {
            server.run(listener, acceptor, None).await.unwrap();
        });
        Self { dir, config, task }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn options(data: &[u8]) -> SessionOptions {
    SessionOptions {
        family: IpFamily::V4,
        udp_bind: Some("127.0.0.1:0".parse().unwrap()),
        offer: Some(TransferOffer {
            kind: Kind::Text,
            name: "message.txt".into(),
            size: data.len() as u64,
        }),
        ..SessionOptions::sender_default()
    }
}
async fn transfer(mut tx: session::TxSession, mut rx: session::RxSession, data: &[u8], out: &Path) {
    let (sent, receipt) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(
            orchestrate::send_text(&mut tx.conn, std::str::from_utf8(data).unwrap(), None),
            orchestrate::receive(
                &mut rx.conn,
                out,
                ReceiveOptions {
                    offer: rx.offer.as_ref(),
                    ..Default::default()
                }
            )
        )
    })
    .await
    .unwrap();
    assert!(sent.unwrap().verified);
    let receipt = receipt.unwrap();
    assert_eq!(receipt.sha256, hash_bytes(data));
    assert_eq!(std::fs::read(receipt.path).unwrap(), data);
    tx.conn.shutdown().await.unwrap();
    rx.conn.shutdown().await.unwrap();
}

#[tokio::test]
async fn pair_then_receive_by_device_and_preserve_pairing_secret() {
    let fixture = Fixture::new().await;
    let a = Identity::load_or_create(&fixture.dir.join("a")).unwrap();
    let b = Identity::load_or_create(&fixture.dir.join("b")).unwrap();
    let ac = Contacts::open(&fixture.dir.join("a")).unwrap();
    let bc = Contacts::open(&fixture.dir.join("b")).unwrap();
    let data = b"device pairing and ID receive";
    let opts = options(data);
    let rxopts = SessionOptions {
        family: IpFamily::V4,
        udp_bind: Some("127.0.0.1:0".parse().unwrap()),
        ..Default::default()
    };
    let code = "1234-acid-acorn-acre";
    let (ready, wait) = tokio::sync::oneshot::channel();
    let (tx, rx) = tokio::time::timeout(Duration::from_secs(60), async {
        tokio::join!(
            session::sender_device_session(&fixture.config, code, &opts, &a, &ac, true, || {
                let _ = ready.send(());
            }),
            async {
                wait.await.unwrap();
                session::receiver_pairing_session(&fixture.config, code, &rxopts, &b, &bc).await
            }
        )
    })
    .await
    .unwrap();
    transfer(
        tx.unwrap(),
        rx.unwrap(),
        data,
        &fixture.dir.join("paired-out"),
    )
    .await;
    let peer_a = bc.lookup(a.device_id()).unwrap();
    let first_secret = *peer_a.secret();
    assert_eq!(ac.lookup(b.device_id()).unwrap().secret(), &first_secret);
    bc.rename(a.device_id(), "My laptop").unwrap();
    assert_eq!(
        bc.resolve("My laptop").unwrap().contact().device_id,
        a.device_id()
    );
    let (ready, wait) = tokio::sync::oneshot::channel();
    let (tx, rx) = tokio::time::timeout(Duration::from_secs(60), async {
        tokio::join!(
            session::sender_dual_session(
                &fixture.config,
                "1235-acid-acorn-acre",
                &opts,
                &a,
                &ac,
                || {
                    let _ = ready.send(());
                }
            ),
            async {
                wait.await.unwrap();
                // 控制连接认证与码分配并行；offline 只在注册尚未完成时重试，次数有界。
                let mut result = None;
                for _ in 0..10 {
                    let attempt = session::receiver_device_session(
                        &fixture.config,
                        &peer_a,
                        &rxopts,
                        &b,
                        &bc,
                    )
                    .await;
                    match attempt {
                        Ok(session) => {
                            result = Some(Ok(session));
                            break;
                        }
                        Err(error) => {
                            result = Some(Err(error));
                            tokio::task::yield_now().await;
                        }
                    }
                }
                result.unwrap()
            }
        )
    })
    .await
    .unwrap();
    transfer(tx.unwrap(), rx.unwrap(), data, &fixture.dir.join("id-out")).await;
    assert_eq!(bc.lookup(a.device_id()).unwrap().secret(), &first_secret);
    assert_eq!(
        bc.lookup(a.device_id()).unwrap().contact().name,
        "My laptop"
    );
}
