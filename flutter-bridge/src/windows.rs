//! Windows-native GUI integration. Core sees only the safe storage interface.
#[allow(unsafe_code)]
mod acl;
#[allow(unsafe_code)]
pub mod files;

use std::sync::{Arc, OnceLock};

#[derive(Debug)]
struct StoragePermissions;
impl hermod_core::identity::StoragePermissions for StoragePermissions {
    fn private_storage_root(&self) -> std::io::Result<Option<std::path::PathBuf>> {
        acl::private_storage_root()
    }
    fn initialize_private(&self, path: &std::path::Path, directory: bool) -> std::io::Result<()> {
        acl::private_acl(path, directory, true)
    }
    fn verify_private(&self, path: &std::path::Path, directory: bool) -> std::io::Result<()> {
        acl::private_acl(path, directory, false)
    }
}

/// Register exactly one safe adapter before any identity/contact access.
pub(crate) fn initialize_storage() -> Result<(), String> {
    static INITIALIZED: OnceLock<Result<(), String>> = OnceLock::new();
    INITIALIZED
        .get_or_init(|| {
            hermod_core::identity::install_storage_permissions(Arc::new(StoragePermissions))
                .map_err(|e| format!("Could not initialize Windows storage adapter: {e}"))
        })
        .clone()
}
