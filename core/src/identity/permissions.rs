//! Safe host-injected private-storage permissions boundary.
//!
//! Windows hosts must install their native backend before opening identity or
//! contacts storage. There is deliberately no permissive or shell fallback.
use std::{
    io,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

/// Native permission operations supplied by the embedding executable.
///
/// Implementations must fail closed if token or descriptor inspection fails.
/// Initialization is called only for a newly-created object; verification must
/// never repair existing permissions. The optional root must come from a trusted
/// OS authority (not an environment variable or caller-selected path).
pub trait StoragePermissions: std::fmt::Debug + Send + Sync + 'static {
    fn private_storage_root(&self) -> io::Result<Option<PathBuf>>;
    fn initialize_private(&self, path: &Path, directory: bool) -> io::Result<()>;
    fn verify_private(&self, path: &Path, directory: bool) -> io::Result<()>;
}

static BACKEND: OnceLock<Arc<dyn StoragePermissions>> = OnceLock::new();

/// Install the process-wide backend once, before any Windows storage access.
///
/// A second installation returns `AlreadyExists`; the original backend is never
/// replaced, including while other threads are performing storage operations.
/// Unix storage uses safe native mode/UID checks and does not need this backend.
pub fn install_storage_permissions(backend: Arc<dyn StoragePermissions>) -> io::Result<()> {
    BACKEND.set(backend).map_err(|_| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "storage permissions backend already installed",
        )
    })
}

#[cfg(windows)]
pub(super) fn backend() -> io::Result<&'static dyn StoragePermissions> {
    BACKEND.get().map(Arc::as_ref).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Windows storage permissions backend not installed",
        )
    })
}
