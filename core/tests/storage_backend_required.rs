//! Separate process: no test backend is installed in this integration binary.
#[cfg(windows)]
#[test]
fn uninitialized_windows_host_fails_before_creating_storage() {
    let path =
        std::env::temp_dir().join(format!("hermod-no-storage-backend-{}", std::process::id()));
    assert!(!path.exists());
    let error = hermod_core::identity::Identity::load_or_create(&path).unwrap_err();
    assert!(error.to_string().contains("backend not installed"));
    assert!(!path.exists());
    assert!(hermod_core::contacts::Contacts::open(&path).is_err());
    assert!(!path.exists());
}
