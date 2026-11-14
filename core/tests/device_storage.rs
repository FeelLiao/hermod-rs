//! 设备身份/联系人的公共 API 验收；不访问私有存储类型。
use hermod_core::{
    contacts::{Contacts, PairingState},
    identity::{self, DeviceId, Identity},
};
use std::{fs, path::PathBuf};

#[path = "support/storage_permissions.rs"]
mod storage_permissions;

struct TempDir(PathBuf);
impl TempDir {
    fn new() -> Self {
        storage_permissions::install();
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let name: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        Self(std::env::temp_dir().join(format!("hermod-device-{name}")))
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        if self.0.exists() {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
}

#[test]
fn identity_persists_and_signatures_bind_domain_and_message() {
    let dir = TempDir::new();
    let identity = Identity::load_or_create(&dir.0).unwrap();
    let reloaded = Identity::load_or_create(&dir.0).unwrap();
    assert_eq!(identity.device_id(), reloaded.device_id());
    assert_eq!(identity.public_key(), reloaded.public_key());
    let public = identity.public_key();
    assert_eq!(
        DeviceId::from_public_key(&public).unwrap(),
        identity.device_id()
    );
    let text = identity.device_id().to_string();
    assert_eq!(text.parse::<DeviceId>().unwrap(), identity.device_id());
    assert_eq!(
        serde_json::to_string(&identity.device_id()).unwrap(),
        format!("\"{text}\"")
    );
    assert_eq!(
        serde_json::from_str::<DeviceId>(&format!("\"{text}\"")).unwrap(),
        identity.device_id()
    );
    assert!(text.to_uppercase().parse::<DeviceId>().is_err());
    assert!("0123".parse::<DeviceId>().is_err());
    let signature = identity.sign(b"pairing", b"message").unwrap();
    identity::verify(&public, b"pairing", b"message", &signature).unwrap();
    assert!(identity::verify(&public, b"other", b"message", &signature).is_err());
    assert!(identity::verify(&public, b"pairing", b"changed", &signature).is_err());
    assert!(identity::verify(&public, b"pairing", b"message", &signature[..63]).is_err());
    assert!(identity::verify(&[0; 33], b"pairing", b"message", &signature).is_err());
    assert!(identity.sign(b"", b"message").is_err());
    let document: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.0.join("device-state/device.json")).unwrap()).unwrap();
    assert!(!format!("{identity:?}").contains(document["private_key"].as_str().unwrap()));
    fs::write(dir.0.join("device-state/device.json"), b"broken").unwrap();
    assert!(Identity::load_or_create(&dir.0).is_err());
    assert_eq!(
        fs::read(dir.0.join("device-state/device.json")).unwrap(),
        b"broken"
    );
}

#[test]
fn pending_confirmation_conflicts_and_secret_redaction() {
    let dir = TempDir::new();
    let identity = Identity::load_or_create(&dir.0).unwrap();
    let contacts = Contacts::open(&dir.0).unwrap();
    let id = identity.device_id();
    let public = identity.public_key();
    let secret = [0x42; 32];
    contacts
        .stage_pairing(id, public, "laptop", "pair-1", secret)
        .unwrap();
    assert!(contacts.find(id).unwrap().is_none());
    assert_eq!(
        contacts.find_any(id).unwrap().unwrap().contact().state,
        PairingState::Pending
    );
    assert_eq!(contacts.list().unwrap()[0].state, PairingState::Pending);
    contacts
        .stage_pairing(id, public, "laptop", "pair-1", secret)
        .unwrap();
    assert!(contacts.confirm_pairing(id, "wrong").is_err());
    contacts.confirm_pairing(id, "pair-1").unwrap();
    contacts.confirm_pairing(id, "pair-1").unwrap();
    let pairing = contacts.lookup(id).unwrap();
    assert_eq!(pairing.secret(), &secret);
    assert!(format!("{pairing:?}").contains("REDACTED"));
    assert!(!format!("{pairing:?}").contains("4242424242"));
    assert!(
        contacts
            .stage_pairing(id, public, "new", "pair-2", [1; 32])
            .is_err()
    );
    assert!(
        contacts
            .stage_pairing(id, public, "new", "pair-1", [1; 32])
            .is_err()
    );
    contacts.rename(id, "desktop").unwrap();
    assert_eq!(contacts.resolve("desktop").unwrap().contact().device_id, id);
    assert_eq!(
        contacts.resolve(&id.to_string()).unwrap().contact().name,
        "desktop"
    );
    let reloaded = Contacts::open(&dir.0).unwrap();
    assert_eq!(reloaded.lookup(id).unwrap().secret(), &secret);
    let lease = contacts.acquire_send_lease().unwrap();
    // 发送租约不占用联系人事务锁。
    assert_eq!(contacts.list().unwrap().len(), 1);
    assert!(matches!(
        reloaded.acquire_send_lease(),
        Err(identity::Error::Busy)
    ));
    drop(lease);
    let lease_after_drop = reloaded.acquire_send_lease().unwrap();
    drop(lease_after_drop);
    contacts.remove(id).unwrap();
    assert!(contacts.find(id).unwrap().is_none());
    fs::write(
        dir.0.join("device-state/contacts.json"),
        br#"{"version":2,"entries":[]}"#,
    )
    .unwrap();
    assert!(contacts.list().is_err());
    assert!(Contacts::open(&dir.0).is_err());
}

#[test]
fn contact_id_key_and_duplicate_names_are_strict() {
    let dir = TempDir::new();
    let peer_dir = TempDir::new();
    let first = Identity::load_or_create(&dir.0).unwrap();
    let second = Identity::load_or_create(&peer_dir.0).unwrap();
    let contacts = Contacts::open(&dir.0).unwrap();
    assert!(
        contacts
            .stage_pairing(
                first.device_id(),
                second.public_key(),
                "peer",
                "pair",
                [1; 32]
            )
            .is_err()
    );
    contacts
        .stage_pairing(
            first.device_id(),
            first.public_key(),
            "same",
            "one",
            [1; 32],
        )
        .unwrap();
    contacts.confirm_pairing(first.device_id(), "one").unwrap();
    contacts
        .stage_pairing(
            second.device_id(),
            second.public_key(),
            "same",
            "two",
            [2; 32],
        )
        .unwrap();
    contacts.confirm_pairing(second.device_id(), "two").unwrap();
    assert!(contacts.resolve("same").is_err());
    assert!(contacts.rename(first.device_id(), "bad\nname").is_err());
    contacts
        .rename(first.device_id(), "00000000000000000000000000000000")
        .unwrap();
    assert!(
        contacts
            .resolve("00000000000000000000000000000000")
            .is_err()
    );
}

#[test]
fn concurrent_identity_initialization_is_single_winner() {
    let dir = TempDir::new();
    // 首先创建安全目录，专门检验锁内重读而不是目录 ACL 初始化竞争。
    let expected = Identity::load_or_create(&dir.0).unwrap().device_id();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let handles: Vec<_> = (0..3)
        .map(|_| {
            let path = dir.0.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                Identity::load_or_create(&path).unwrap().device_id()
            })
        })
        .collect();
    for handle in handles {
        assert_eq!(handle.join().unwrap(), expected);
    }
}

#[cfg(windows)]
#[test]
fn injected_permission_denial_fails_closed_without_repair() {
    let dir = TempDir::new();
    Identity::load_or_create(&dir.0).unwrap();
    let path = dir.0.join("device-state/device.json");
    let original = fs::read(&path).unwrap();
    // Explicit safe test double; real native ACL tampering is tested by CLI.
    storage_permissions::deny(&path);
    assert!(Identity::load_or_create(&dir.0).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
}

#[test]
fn unsupported_or_mismatched_identity_is_never_rotated() {
    let dir = TempDir::new();
    let original = Identity::load_or_create(&dir.0).unwrap().device_id();
    let path = dir.0.join("device-state/device.json");
    let bytes = fs::read(&path).unwrap();
    let stored: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    for (field, invalid) in [
        ("version", serde_json::json!(99)),
        (
            "device_id",
            serde_json::json!("00000000000000000000000000000000"),
        ),
        ("private_key", serde_json::json!("00".repeat(32))),
    ] {
        let mut modified = stored.clone();
        modified[field] = invalid;
        let modified = serde_json::to_vec(&modified).unwrap();
        fs::write(&path, &modified).unwrap();
        assert!(Identity::load_or_create(&dir.0).is_err());
        assert_eq!(fs::read(&path).unwrap(), modified);
    }
    fs::write(&path, bytes).unwrap();
    assert_eq!(
        Identity::load_or_create(&dir.0).unwrap().device_id(),
        original
    );
}

#[cfg(unix)]
#[test]
fn unix_permissions_and_symlinks_fail_closed() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = TempDir::new();
    Identity::load_or_create(&dir.0).unwrap();
    assert_eq!(
        fs::metadata(dir.0.join("device-state"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    let file = dir.0.join("device-state/device.json");
    assert_eq!(
        fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(Identity::load_or_create(&dir.0).is_err());
    fs::set_permissions(&file, fs::Permissions::from_mode(0o4600)).unwrap();
    assert!(Identity::load_or_create(&dir.0).is_err());
    fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
    let alias = dir.0.join("linked");
    symlink(&dir.0, &alias).unwrap();
    assert!(Identity::load_or_create(&alias).is_err());
}
