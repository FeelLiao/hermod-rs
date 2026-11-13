//! 真实 TLS 服务：失败的设备候选不影响传输码，码获胜后撤销 ID 门。
use hermod_core::{
    config::Config,
    contacts::Contacts,
    identity::Identity,
    options::IpFamily,
    server::{MemoryStore, Server, ServerLimits, cert},
    session::{self, SessionOptions},
    signal::{
        client,
        device::{self, DeviceControl, DeviceError, DeviceMessage},
    },
    transport::network,
};
use std::{path::PathBuf, sync::Arc, time::Duration};

struct Fixture {
    dir: PathBuf,
    config: Config,
    task: tokio::task::JoinHandle<()>,
}
impl Fixture {
    async fn new() -> Self {
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let dir = std::env::temp_dir().join(format!(
            "hermod-dual-negative-{}",
            hermod_core::transfer::hex(&random)
        ));
        let certificate =
            cert::ensure(&dir.join("server"), cert::CERT_AUTO_RENEW_THRESHOLD).unwrap();
        let listener = network::bind_tcp(0, IpFamily::V4).unwrap();
        let url = format!("wss://127.0.0.1:{}", listener.local_addr().unwrap().port());
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
            Arc::new(MemoryStore::new(100)),
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

#[tokio::test]
async fn rejected_device_candidates_preserve_code_and_code_winner_unregisters_id() {
    let fixture = Fixture::new().await;
    let a = Identity::load_or_create(&fixture.dir.join("a")).unwrap();
    let b = Identity::load_or_create(&fixture.dir.join("b")).unwrap();
    let ac = Contacts::open(&fixture.dir.join("a")).unwrap();
    let bc = Contacts::open(&fixture.dir.join("b")).unwrap();
    let opts = SessionOptions {
        family: IpFamily::V4,
        udp_bind: Some("127.0.0.1:0".parse().unwrap()),
        ..Default::default()
    };
    for wrong_secret in [false, true] {
        if wrong_secret {
            ac.stage_pairing(b.device_id(), b.public_key(), "", "wrong-secret", [1; 32])
                .unwrap();
            ac.confirm_pairing(b.device_id(), "wrong-secret").unwrap();
            bc.stage_pairing(a.device_id(), a.public_key(), "", "wrong-secret", [2; 32])
                .unwrap();
            bc.confirm_pairing(a.device_id(), "wrong-secret").unwrap();
        }
        let code = if wrong_secret {
            "1402-acid-acorn-acre"
        } else {
            "1401-acid-acorn-acre"
        };
        let (ready, wait) = tokio::sync::oneshot::channel();
        let sender = session::sender_dual_session(&fixture.config, code, &opts, &a, &ac, || {
            let _ = ready.send(());
        });
        let receiver = async {
            wait.await.unwrap();
            let mut caller = DeviceControl::connect(&fixture.config, IpFamily::V4, &b, false)
                .await
                .unwrap();
            let (url, pin) = session::expected_spki(&fixture.config).unwrap();
            let mut registered = false;
            // 码 ready 不代表控制注册已完成；只对明确 offline 结果重试。
            for channel in 1500..1520 {
                let mut ws = client::connect_pinned(&url, pin).await.unwrap();
                let (_, token) = device::allocate_channel(&mut ws, channel).await.unwrap();
                let call = caller.call(a.device_id(), channel, token).await.unwrap();
                let mut pending = false;
                loop {
                    match caller.next_event().await {
                        Ok(DeviceMessage::CallPending { .. }) => {
                            pending = true;
                            continue;
                        }
                        Err(DeviceError::Rejected(reason)) if reason == "unavailable" => break,
                        Ok(DeviceMessage::Declined { call_id }) if call_id == call => {
                            if !pending {
                                break;
                            }
                            assert!(!wrong_secret);
                            registered = true;
                            break;
                        }
                        Ok(DeviceMessage::Accepted { call_id }) if call_id == call => {
                            assert!(wrong_secret);
                            registered = true;
                            // 同一合法设备身份但错误配对秘密：CPace 后必然认证失败。
                            client::wait_ready(&mut ws).await.unwrap();
                            let password = "02".repeat(32);
                            let (pake, public) = hermod_core::crypto::cpace::init(
                                &password,
                                channel,
                                hermod_core::crypto::cpace::Role::Receiver,
                            )
                            .unwrap();
                            client::send_blob(&mut ws, channel, &public).await.unwrap();
                            let peer: [u8; 65] = client::recv_blob(&mut ws)
                                .await
                                .unwrap()
                                .try_into()
                                .unwrap();
                            let key = pake.finish(&peer).unwrap();
                            // 发一个无法通过对端 AEAD 的候选 hello，促使其明确拒绝。
                            let blob = hermod_core::crypto::aead::seal(
                                &key,
                                b"wrong-auth-aad",
                                b"wrong-secret",
                            )
                            .unwrap();
                            client::send_blob(&mut ws, channel, &blob).await.unwrap();
                            // Accept 已结束控制呼叫；认证失败通过候选频道断开通知。
                            tokio::time::timeout(Duration::from_secs(5), async {
                                loop {
                                    if client::recv_blob(&mut ws).await.is_err() {
                                        break;
                                    }
                                }
                            })
                            .await
                            .expect("failed authentication did not close candidate channel");
                            break;
                        }
                        event => panic!("unexpected control event: {event:?}"),
                    }
                }
                if registered {
                    break;
                }
                tokio::task::yield_now().await;
            }
            assert!(
                registered,
                "device listener did not register within bounded retries"
            );
            session::receiver_session_with(&fixture.config, code, &opts).await
        };
        let (tx, rx) = tokio::time::timeout(
            Duration::from_secs(40),
            futures_util::future::join(sender, receiver),
        )
        .await
        .unwrap();
        let _tx = tx.unwrap();
        let _rx = rx.unwrap();
        let mut caller = DeviceControl::connect(&fixture.config, IpFamily::V4, &b, false)
            .await
            .unwrap();
        let (url, pin) = session::expected_spki(&fixture.config).unwrap();
        let mut ws = client::connect_pinned(&url, pin).await.unwrap();
        let channel = if wrong_secret { 1602 } else { 1601 };
        let (_, token) = device::allocate_channel(&mut ws, channel).await.unwrap();
        let call = caller.call(a.device_id(), channel, token).await.unwrap();
        let result = tokio::time::timeout(Duration::from_secs(5), caller.next_event())
            .await
            .unwrap();
        assert!(
            matches!(&result, Ok(DeviceMessage::Declined { call_id }) if *call_id == call),
            "expected offline Declined after code victory, got {result:?}"
        );
    }
}
