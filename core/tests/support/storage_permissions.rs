//! Explicit safe test double, NOT a native ACL adapter or ACL acceptance test.
//! Windows native descriptor/tamper coverage lives in the CLI and GUI adapters.
#[cfg(windows)]
mod windows {
    use hermod_core::identity::{StoragePermissions, install_storage_permissions};
    use std::{
        collections::HashSet,
        io,
        path::{Path, PathBuf},
        sync::{Arc, Mutex, OnceLock},
    };
    #[derive(Debug, Default)]
    struct TestPermissions {
        denied: Mutex<HashSet<PathBuf>>,
    }
    impl StoragePermissions for TestPermissions {
        fn private_storage_root(&self) -> io::Result<Option<PathBuf>> {
            Ok(None)
        }
        fn initialize_private(&self, path: &Path, directory: bool) -> io::Result<()> {
            self.verify_private(path, directory)
        }
        fn verify_private(&self, path: &Path, _directory: bool) -> io::Result<()> {
            if !path.starts_with(std::env::temp_dir()) || self.denied.lock().unwrap().contains(path)
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "test backend denied path",
                ));
            }
            Ok(())
        }
    }
    static BACKEND: OnceLock<Arc<TestPermissions>> = OnceLock::new();
    pub(super) fn install() {
        BACKEND.get_or_init(|| {
            let backend = Arc::new(TestPermissions::default());
            install_storage_permissions(backend.clone()).unwrap();
            backend
        });
    }
    #[allow(dead_code)]
    pub(super) fn deny(path: &Path) {
        install();
        BACKEND
            .get()
            .unwrap()
            .denied
            .lock()
            .unwrap()
            .insert(path.to_owned());
    }
}
pub fn install() {
    #[cfg(windows)]
    windows::install();
}
#[cfg(windows)]
#[allow(dead_code)]
pub fn deny(path: &std::path::Path) {
    windows::deny(path);
}
