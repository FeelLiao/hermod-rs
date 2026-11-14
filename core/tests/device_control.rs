//! 真实 TLS 服务上的设备控制协议集成测试。
use futures_util::{SinkExt, StreamExt};
use hermod_core::{
    config::Config,
    identity::Identity,
    options::IpFamily,
    signal::{
        client,
        device::{
            DeviceControl, DeviceError, DeviceMessage, REGISTER_DOMAIN, VERSION, allocate_channel,
            registration_message,
        },
    },
};
use tokio_tungstenite::tungstenite::Message as Frame;
#[path = "support/storage_permissions.rs"]
mod storage_permissions;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn unhex<const N: usize>(value: &str) -> Option<[u8; N]> {
    if value.len() != N * 2 {
        return None;
    }
    let mut result = [0; N];
    for (i, b) in result.iter_mut().enumerate() {
        *b = u8::from_str_radix(&value[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(result)
}
#[tokio::test]
async fn authenticated_call_requires_channel_owner_and_routes_terminal_events() {
    use hermod_core::server::{MemoryStore, Server, ServerLimits};
    use std::{sync::Arc, time::Duration};
    let mut random = [0; 16];
    getrandom::fill(&mut random).unwrap();
    let dir = std::env::temp_dir().join(format!("hermod-device-test-{}", hex(&random)));
    let cert = hermod_core::server::cert::ensure(&dir.join("server"), Duration::ZERO).unwrap();
    storage_permissions::install();
    let target_identity = Identity::load_or_create(&dir.join("target")).unwrap();
    let caller_identity = Identity::load_or_create(&dir.join("caller")).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("wss://{}", listener.local_addr().unwrap());
    let server = Server::new(
        ServerLimits {
            ttl: Duration::from_secs(60),
            max_blobs_per_channel: 10,
            max_cpace_failures: 3,
        },
        Arc::new(MemoryStore::new(100)),
        100.0,
        100.0,
        cert.cert_pem.clone(),
    );
    let task = tokio::spawn(server.run(
        listener,
        hermod_core::server::tls_acceptor(&cert).unwrap(),
        None,
    ));
    let config = Config {
        server_url: url.clone(),
        trusted_servers: std::collections::BTreeMap::from([(
            url.clone(),
            hex(&cert.spki_fingerprint),
        )]),
        ..Config::default()
    };
    let mut target = DeviceControl::connect(&config, IpFamily::Auto, &target_identity, true)
        .await
        .unwrap();
    assert!(
        DeviceControl::connect(&config, IpFamily::Auto, &target_identity, true)
            .await
            .is_err()
    );
    // 冒充 ID 的有效签名仍不能绕过公钥哈希校验。
    let endpoint = format!("{url}/device");
    let mut invalid = client::connect_endpoint_pinned_with_family(
        &endpoint,
        cert.spki_fingerprint,
        IpFamily::Auto,
    )
    .await
    .unwrap();
    let challenge: DeviceMessage =
        serde_json::from_str(invalid.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
    let DeviceMessage::Challenge { challenge, .. } = challenge else {
        panic!("missing challenge")
    };
    let challenge = unhex::<32>(&challenge).unwrap();
    let sig = caller_identity
        .sign(
            REGISTER_DOMAIN,
            &registration_message(&challenge, target_identity.device_id(), true),
        )
        .unwrap();
    invalid
        .send(Frame::text(
            serde_json::to_string(&DeviceMessage::Register {
                version: VERSION,
                id: target_identity.device_id(),
                pubkey: hex(&caller_identity.public_key()),
                sig: hex(&sig),
                register: true,
            })
            .unwrap(),
        ))
        .await
        .unwrap();
    assert!(!matches!(
        tokio::time::timeout(std::time::Duration::from_secs(2), invalid.next()).await,
        Ok(Some(Ok(Frame::Text(_))))
    ));
    // 先前连接的签名不能用于新服务器挑战，即使公钥和 ID 一致。
    let mut replay = client::connect_endpoint_pinned_with_family(
        &endpoint,
        cert.spki_fingerprint,
        IpFamily::Auto,
    )
    .await
    .unwrap();
    let _fresh_challenge = replay.next().await.unwrap().unwrap();
    let stale_sig = caller_identity
        .sign(
            REGISTER_DOMAIN,
            &registration_message(&challenge, caller_identity.device_id(), false),
        )
        .unwrap();
    replay
        .send(Frame::text(
            serde_json::to_string(&DeviceMessage::Register {
                version: VERSION,
                id: caller_identity.device_id(),
                pubkey: hex(&caller_identity.public_key()),
                sig: hex(&stale_sig),
                register: false,
            })
            .unwrap(),
        ))
        .await
        .unwrap();
    assert!(!matches!(
        tokio::time::timeout(Duration::from_secs(2), replay.next()).await,
        Ok(Some(Ok(Frame::Text(_))))
    ));
    // 非在线目标返回拒绝，不影响已认证 caller 继续发起呼叫。
    let mut caller = DeviceControl::connect(&config, IpFamily::Auto, &caller_identity, false)
        .await
        .unwrap();
    let offline_call = caller
        .call(caller_identity.device_id(), 1900, "00".repeat(32))
        .await
        .unwrap();
    assert_eq!(
        caller.next_event().await.unwrap(),
        DeviceMessage::Declined {
            call_id: offline_call
        }
    );
    let mut legacy = client::connect_pinned(&url, cert.spki_fingerprint)
        .await
        .unwrap();
    let (_, token) = allocate_channel(&mut legacy, 1919).await.unwrap();
    caller
        .call(target_identity.device_id(), 1919, "00".repeat(32))
        .await
        .unwrap();
    assert!(
        matches!(caller.next_event().await, Err(DeviceError::Rejected(code)) if code == "channel_not_owned")
    );
    let call_id = caller
        .call(target_identity.device_id(), 1919, token.clone())
        .await
        .unwrap();
    assert_eq!(
        caller.next_event().await.unwrap(),
        DeviceMessage::CallPending {
            call_id: call_id.clone()
        }
    );
    assert_eq!(
        target.next_event().await.unwrap(),
        DeviceMessage::Incoming {
            caller_id: caller_identity.device_id(),
            call_id: call_id.clone(),
            channel_id: 1919
        }
    );
    target.decline(&call_id).await.unwrap();
    assert_eq!(
        caller.next_event().await.unwrap(),
        DeviceMessage::Declined {
            call_id: call_id.clone()
        }
    );
    assert_eq!(
        target.next_event().await.unwrap(),
        DeviceMessage::Declined { call_id }
    );
    caller
        .call(target_identity.device_id(), 1919, token)
        .await
        .unwrap();
    assert!(
        matches!(caller.next_event().await, Err(DeviceError::Rejected(code)) if code == "channel_not_owned")
    );
    let mut second_legacy = client::connect_pinned(&url, cert.spki_fingerprint)
        .await
        .unwrap();
    let (_, token) = allocate_channel(&mut second_legacy, 1920).await.unwrap();
    let disconnected_call = caller
        .call(target_identity.device_id(), 1920, token)
        .await
        .unwrap();
    assert!(matches!(
        caller.next_event().await.unwrap(),
        DeviceMessage::CallPending { .. }
    ));
    assert!(matches!(
        target.next_event().await.unwrap(),
        DeviceMessage::Incoming { .. }
    ));
    // 已认证但无关连接不能接受或取消别人的呼叫。
    let mut stranger = DeviceControl::connect(&config, IpFamily::Auto, &caller_identity, false)
        .await
        .unwrap();
    stranger.accept(&disconnected_call).await.unwrap();
    assert!(stranger.next_event().await.is_err());
    drop(target);
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(2), caller.next_event())
            .await
            .unwrap()
            .unwrap(),
        DeviceMessage::Cancelled {
            call_id: disconnected_call
        }
    );
    let mut target = DeviceControl::connect(&config, IpFamily::Auto, &target_identity, true)
        .await
        .unwrap();
    let mut third_legacy = client::connect_pinned(&url, cert.spki_fingerprint)
        .await
        .unwrap();
    let (_, token) = allocate_channel(&mut third_legacy, 1921).await.unwrap();
    let call_id = caller
        .call(target_identity.device_id(), 1921, token)
        .await
        .unwrap();
    assert!(matches!(
        caller.next_event().await.unwrap(),
        DeviceMessage::CallPending { .. }
    ));
    assert!(matches!(
        target.next_event().await.unwrap(),
        DeviceMessage::Incoming { .. }
    ));
    target.accept(&call_id).await.unwrap();
    assert_eq!(
        caller.next_event().await.unwrap(),
        DeviceMessage::Accepted {
            call_id: call_id.clone()
        }
    );
    assert_eq!(
        target.next_event().await.unwrap(),
        DeviceMessage::Accepted { call_id }
    );
    drop(target);
    drop(second_legacy);
    drop(third_legacy);
    drop(caller);
    drop(legacy);
    task.abort();
    let _ = task.await;
    std::fs::remove_dir_all(&dir).unwrap();
}
