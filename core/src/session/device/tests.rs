//! 设备认证 transcript 的独立负向用例。

use super::*;

fn hello(role: &str) -> Hello {
    Hello {
        version: 1,
        mode: "pair".into(),
        role: role.into(),
        id: "0123456789abcdef0123456789abcdef".into(),
        public_key: vec![2; 33],
        nonce: [7; 32],
        endpoint: vec![b'H', b'E', 3, 2, 42],
    }
}

fn status() -> PairingStatus {
    PairingStatus {
        pairing_id: None,
        proof: None,
        pending: false,
    }
}

#[test]
fn transcript_binds_every_security_relevant_field() {
    let tx = hello("tx");
    let rx = hello("rx");
    let baseline = canonical(17, &[9; 32], &tx, &rx, &status(), &status()).unwrap();
    assert_ne!(
        baseline,
        canonical(18, &[9; 32], &tx, &rx, &status(), &status()).unwrap()
    );
    assert_ne!(
        baseline,
        canonical(17, &[8; 32], &tx, &rx, &status(), &status()).unwrap()
    );
    assert_ne!(
        baseline,
        canonical(17, &[9; 32], &rx, &tx, &status(), &status()).unwrap()
    );
    for field in 0..7 {
        let mut changed = hello("tx");
        match field {
            0 => changed.version = 2,
            1 => changed.mode = "device".into(),
            2 => changed.role = "rx".into(),
            3 => changed.id.push('0'),
            4 => changed.public_key[1] ^= 1,
            5 => changed.nonce[0] ^= 1,
            _ => changed.endpoint.push(99),
        }
        assert_ne!(
            baseline,
            canonical(17, &[9; 32], &changed, &rx, &status(), &status()).unwrap()
        );
    }
    let pending = PairingStatus {
        pairing_id: Some("recovery-id".into()),
        proof: Some([5; 32]),
        pending: true,
    };
    assert_ne!(
        baseline,
        canonical(17, &[9; 32], &tx, &rx, &pending, &status()).unwrap()
    );
}

#[test]
fn authentication_kdfs_are_channel_and_domain_separated() {
    let key = [6; 32];
    let auth = derive(&key, 17, b"hermod-device-aead-v1");
    assert_ne!(auth, derive(&key, 18, b"hermod-device-aead-v1"));
    assert_ne!(auth, derive(&key, 17, b"hermod-device-exporter-v1"));
    assert_ne!(auth, derive(&key, 17, b"hermod-pairing-secret-v1"));
}
