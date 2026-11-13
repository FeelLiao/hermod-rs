//! 公共 API：真实中继下显式码配对、幂等重配及确认中断恢复。
mod common;

use hermod_core::{
    contacts::{Contacts, PairingState},
    crypto::wordlist,
    identity::Identity,
    session::{self, SessionOptions},
};
use std::time::Duration;

#[tokio::test]
async fn leaked_pairing_secret_does_not_authorize_a_different_device_identity() {
    use hermod_core::{crypto::cpace::Role, session::device, signal::client};
    let a_dir = common::temp_dir("pin-owner");
    let b_dir = common::temp_dir("pin-expected");
    let c_dir = common::temp_dir("pin-attacker");
    let a = Identity::load_or_create(&a_dir).unwrap();
    let b = Identity::load_or_create(&b_dir).unwrap();
    let c = Identity::load_or_create(&c_dir).unwrap();
    let ac = Contacts::open(&a_dir).unwrap();
    let cc = Contacts::open(&c_dir).unwrap();
    let secret = [19; 32];
    let pairing_id = "leaked-secret-test";
    ac.stage_pairing(
        b.device_id(),
        b.public_key(),
        "expected",
        pairing_id,
        secret,
    )
    .unwrap();
    ac.confirm_pairing(b.device_id(), pairing_id).unwrap();
    cc.stage_pairing(a.device_id(), a.public_key(), "owner", pairing_id, secret)
        .unwrap();
    cc.confirm_pairing(a.device_id(), pairing_id).unwrap();
    let expected_b = ac.lookup(b.device_id()).unwrap();
    let expected_a = cc.lookup(a.device_id()).unwrap();
    let (config, relay) = common::start_relay().await;
    let opts = SessionOptions {
        udp_bind: Some("[::1]:0".parse().unwrap()),
        ..Default::default()
    };
    let (url, pin) = session::expected_spki(&config).unwrap();
    let password = device::pairing_password(&expected_b);
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let owner = async {
        let mut ws = client::connect_pinned(&url, pin).await.unwrap();
        let (key, observed) = client::sender_handshake_with_ready(&mut ws, 1234, &password, || {
            let _ = ready_tx.send(());
        })
        .await
        .unwrap();
        device::authenticate_device_candidate(
            &mut ws,
            1234,
            &key,
            observed,
            Role::Sender,
            &opts,
            &a,
            &ac,
            Some(&expected_b),
        )
        .await
    };
    let attacker = async {
        ready_rx.await.unwrap();
        let mut ws = client::connect_pinned(&url, pin).await.unwrap();
        let (key, observed) = client::receiver_handshake(&mut ws, 1234, &password)
            .await
            .unwrap();
        device::authenticate_device_candidate(
            &mut ws,
            1234,
            &key,
            observed,
            Role::Receiver,
            &opts,
            &c,
            &cc,
            Some(&expected_a),
        )
        .await
    };
    tokio::pin!(owner, attacker);
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        tokio::select! {
            biased;
            result = &mut owner => result,
            _ = &mut attacker => panic!("attacker must not complete before owner rejects its identity"),
        }
    }).await.unwrap();
    assert!(
        matches!(result, Err(session::SessionError::Authentication(message)) if message.contains("pinned device identity"))
    );
    assert!(ac.find(c.device_id()).unwrap().is_none());
    let saved = ac.lookup(b.device_id()).unwrap();
    assert_eq!(saved.secret(), &secret);
    assert_eq!(saved.contact().pairing_id, pairing_id);
    relay.abort();
    common::cleanup_dir(&a_dir);
    common::cleanup_dir(&b_dir);
    common::cleanup_dir(&c_dir);
}

#[tokio::test]
async fn explicit_pairing_is_confirmed_before_session_and_preserves_secret() {
    let tx_dir = common::temp_dir("device-pair-tx");
    let rx_dir = common::temp_dir("device-pair-rx");
    let tx_id = Identity::load_or_create(&tx_dir).unwrap();
    let rx_id = Identity::load_or_create(&rx_dir).unwrap();
    let tx_contacts = Contacts::open(&tx_dir).unwrap();
    let rx_contacts = Contacts::open(&rx_dir).unwrap();
    let opts = SessionOptions {
        udp_bind: Some("[::1]:0".parse().unwrap()),
        ..Default::default()
    };
    let mut previous = None;
    for _ in 0..2 {
        let (config, relay) = common::start_relay().await;
        let mut rng = rand_core::UnwrapErr(getrandom::SysRng);
        let (_, code) = wordlist::generate_transfer_code(&mut rng, 3);
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let sender = session::sender_device_session(
            &config,
            &code,
            &opts,
            &tx_id,
            &tx_contacts,
            true,
            || {
                let _ = ready_tx.send(());
            },
        );
        let receiver = async {
            ready_rx.await.map_err(|_| {
                session::SessionError::Authentication(
                    "sender exited before readiness; inspect sender result".into(),
                )
            })?;
            session::receiver_pairing_session(&config, &code, &opts, &rx_id, &rx_contacts).await
        };
        let (tx, rx) = tokio::time::timeout(
            Duration::from_secs(30),
            futures_util::future::join(sender, receiver),
        )
        .await
        .unwrap();
        let _tx = tx.unwrap();
        let _rx = rx.unwrap();
        let a = tx_contacts.lookup(rx_id.device_id()).unwrap();
        let b = rx_contacts.lookup(tx_id.device_id()).unwrap();
        assert_eq!(a.contact().state, PairingState::Active);
        assert_eq!(b.contact().state, PairingState::Active);
        assert_eq!(a.contact().pairing_id, b.contact().pairing_id);
        assert_eq!(a.secret(), b.secret());
        let snapshot = (a.contact().pairing_id.clone(), *a.secret());
        if let Some(previous) = &previous {
            assert_eq!(previous, &snapshot);
        }
        previous = Some(snapshot);
        relay.abort();
    }
    common::cleanup_dir(&tx_dir);
    common::cleanup_dir(&rx_dir);
}

#[tokio::test]
async fn interrupted_pairing_recovers_pending_and_active_without_rotation() {
    for (tx_state, rx_state) in [(1, 0), (0, 1), (1, 1), (2, 1), (1, 2)] {
        // 0=尚未落盘，1=pending，2=已 confirm 但可能没收到最终 ACK。
        let tx_dir = common::temp_dir("recover-pair-tx");
        let rx_dir = common::temp_dir("recover-pair-rx");
        let tx_id = Identity::load_or_create(&tx_dir).unwrap();
        let rx_id = Identity::load_or_create(&rx_dir).unwrap();
        let tx_contacts = Contacts::open(&tx_dir).unwrap();
        let rx_contacts = Contacts::open(&rx_dir).unwrap();
        let secret = [41; 32];
        let pairing_id = "interrupted-pairing-id";
        for (state, contacts, peer) in [
            (tx_state, &tx_contacts, &rx_id),
            (rx_state, &rx_contacts, &tx_id),
        ] {
            if state != 0 {
                contacts
                    .stage_pairing(peer.device_id(), peer.public_key(), "", pairing_id, secret)
                    .unwrap();
            }
            if state == 2 {
                contacts
                    .confirm_pairing(peer.device_id(), pairing_id)
                    .unwrap();
            }
        }
        let (config, relay) = common::start_relay().await;
        let opts = SessionOptions {
            udp_bind: Some("[::1]:0".parse().unwrap()),
            ..Default::default()
        };
        let mut rng = rand_core::UnwrapErr(getrandom::SysRng);
        let (_, code) = wordlist::generate_transfer_code(&mut rng, 3);
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let sender = session::sender_device_session(
            &config,
            &code,
            &opts,
            &tx_id,
            &tx_contacts,
            true,
            || {
                let _ = ready_tx.send(());
            },
        );
        let receiver = async {
            ready_rx.await.map_err(|_| {
                session::SessionError::Authentication(
                    "sender exited before readiness; inspect sender result".into(),
                )
            })?;
            session::receiver_pairing_session(&config, &code, &opts, &rx_id, &rx_contacts).await
        };
        let (tx, rx) = tokio::time::timeout(
            Duration::from_secs(30),
            futures_util::future::join(sender, receiver),
        )
        .await
        .unwrap();
        let _tx = tx.unwrap();
        let _rx = rx.unwrap();
        for (contacts, peer) in [(&tx_contacts, &rx_id), (&rx_contacts, &tx_id)] {
            let saved = contacts.lookup(peer.device_id()).unwrap();
            assert_eq!(saved.contact().state, PairingState::Active);
            assert_eq!(saved.contact().pairing_id, pairing_id);
            assert_eq!(saved.secret(), &secret);
        }
        relay.abort();
        common::cleanup_dir(&tx_dir);
        common::cleanup_dir(&rx_dir);
    }
}
