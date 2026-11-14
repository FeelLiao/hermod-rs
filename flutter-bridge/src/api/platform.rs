//! Platform file authorization: opaque selections, never OS objects over FRB.
use super::controller;
use crate::gui_api::{GuiError, GuiErrorCode, SelectionDto};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
};

// Only platform-picker-created files enter this registry. Ordinary select_source
// paths are never owned, even if their basename resembles a staging filename.
static OWNED_SOURCES: LazyLock<Mutex<HashMap<String, (PathBuf, PathBuf)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn release_private_file(root: &Path, path: &Path, export: bool) -> Result<(), GuiError> {
    let root = root
        .canonicalize()
        .map_err(|e| platform_error(e.to_string()))?;
    let parent = path
        .parent()
        .ok_or_else(|| platform_error("Invalid staging path".into()))?;
    if parent
        .canonicalize()
        .map_err(|e| platform_error(e.to_string()))?
        != root
    {
        return Err(platform_error(
            "Staging path is outside its private directory".into(),
        ));
    }
    if export {
        let id = path
            .file_name()
            .and_then(|s| s.to_str())
            .and_then(|s| s.strip_suffix(".export"));
        if !id.is_some_and(|id| id.len() == 32 && id.bytes().all(|c| c.is_ascii_hexdigit())) {
            return Err(platform_error("Invalid export staging filename".into()));
        }
    }
    // Reject symlinks/reparse redirects, including links into another private
    // area such as inbox. Remove the exact validated file, never a directory.
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(platform_error(e.to_string())),
        Ok(metadata) if !metadata.file_type().is_file() => {
            return Err(platform_error("Staging path is not a regular file".into()));
        }
        Ok(_) => {}
    }
    let canonical = path
        .canonicalize()
        .map_err(|e| platform_error(e.to_string()))?;
    if canonical.parent() != Some(root.as_path()) {
        return Err(platform_error(
            "Staging path resolves outside its private directory".into(),
        ));
    }
    std::fs::remove_file(path).map_err(|e| platform_error(e.to_string()))
}

/// Release only a Windows picker-owned snapshot, identified by opaque selection ID.
/// User files passed to select_source/select_named_source are never registered.
pub fn release_windows_source(selection_id: String) -> Result<(), GuiError> {
    if let Ok(gui) = controller() {
        gui.ensure_idle()?;
    }
    let mut owned = OWNED_SOURCES
        .lock()
        .map_err(|_| platform_error("Source ownership lock poisoned".into()))?;
    if let Some((root, path)) = owned.get(&selection_id) {
        release_private_file(root, path, false)?;
        owned.remove(&selection_id);
    }
    Ok(())
}

/// Release a private Android export copy, never an inbox item or external path.
pub fn release_export(path: String) -> Result<(), GuiError> {
    release_private_file(&controller()?.export_cache(), Path::new(&path), true)
}

struct ExportScratch {
    path: PathBuf,
    keep: bool,
}
impl Drop for ExportScratch {
    fn drop(&mut self) {
        // Constructed only from our freshly generated private export path. RAII
        // also runs on unwind, unlike cleanup after the broker call returns.
        if self.keep {
            return;
        }
        if let Err(error) = std::fs::remove_file(&self.path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(%error, "Could not clean private export staging file");
        }
    }
}

fn platform_error(message: String) -> GuiError {
    GuiError {
        code: GuiErrorCode::Internal,
        message,
        retryable: true,
    }
}

pub fn windows_support_directory() -> Result<String, GuiError> {
    #[cfg(windows)]
    {
        crate::windows::files::support_directory()
            .map(|p| p.to_string_lossy().into_owned())
            .map_err(platform_error)
    }
    #[cfg(not(windows))]
    Err(platform_error(
        "Windows file authorization is unavailable on this platform".into(),
    ))
}

/// Cancel only platform file copying; observing/cancelling files is not task cancellation.
pub fn cancel_file_operation() -> bool {
    #[cfg(windows)]
    {
        crate::windows::files::cancel_file_operation()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

pub fn windows_pick_source(window_handle: u64) -> Result<Option<SelectionDto>, GuiError> {
    #[cfg(windows)]
    {
        let gui = controller()?;
        gui.ensure_idle()?;
        let cache = crate::windows::files::support_directory()
            .map_err(platform_error)?
            .join("authorized-sources");
        std::fs::create_dir_all(&cache).map_err(|e| platform_error(e.to_string()))?;
        let Some(path) = crate::windows::files::pick_to_private(window_handle, cache.clone())
            .map_err(platform_error)?
        else {
            return Ok(None);
        };
        let mut owned = OWNED_SOURCES.lock().map_err(|_| {
            let _ = release_private_file(&cache, &path.path, false);
            platform_error("Source ownership lock poisoned".into())
        })?;
        match gui.select_file_named(&path.path, path.name) {
            Ok(selection) => {
                owned.insert(selection.id.clone(), (cache, path.path));
                Ok(Some(selection))
            }
            Err(error) => {
                if let Err(cleanup) = release_private_file(&cache, &path.path, false) {
                    tracing::warn!(message = %cleanup.message, "Could not clean rejected source");
                }
                Err(error)
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = window_handle;
        Err(platform_error(
            "Windows file authorization is unavailable".into(),
        ))
    }
}

/// Produce a private, opaque export staging copy for Android's SAF writer.
/// Never call with an external destination path or expose the internal inbox path.
pub fn prepare_export(id: String) -> Result<String, GuiError> {
    let gui = controller()?;
    gui.ensure_idle()?;
    let root = gui.export_cache();
    std::fs::create_dir_all(&root).map_err(|e| platform_error(e.to_string()))?;
    let path = root.join(format!("{}.export", crate::app::storage::new_id()));
    // Establish ownership atomically before installing cleanup: an unlikely ID
    // collision must never cause us to remove a pre-existing file.
    let output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| platform_error(e.to_string()))?;
    let mut scratch = ExportScratch {
        path: path.clone(),
        keep: false,
    };
    // Declare the handle after the guard so unwind closes it before deletion.
    let mut output = output;
    gui.export(&id, &mut output)?;
    output
        .sync_all()
        .map_err(|e| platform_error(e.to_string()))?;
    drop(output);
    // Ownership moves to the caller, who must release_export in a finally block.
    scratch.keep = true;
    Ok(path.to_string_lossy().into_owned())
}

pub fn windows_export_received(window_handle: u64, id: String) -> Result<bool, GuiError> {
    let gui = controller()?;
    let item = gui.received(&id)?;
    let path = std::path::PathBuf::from(prepare_export(id)?);
    let _scratch = ExportScratch {
        path: path.clone(),
        keep: false,
    };
    #[cfg(windows)]
    let result = crate::windows::files::export_from_private(window_handle, path.clone(), item.name)
        .map_err(platform_error);
    #[cfg(not(windows))]
    let result = {
        let _ = (window_handle, item);
        Err(platform_error(
            "Windows file authorization is unavailable".into(),
        ))
    };
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_release_rejects_external_inbox_and_invalid_names() {
        let root =
            std::env::temp_dir().join(format!("hermod-staging-{}", crate::app::storage::new_id()));
        let cache = root.join("export-cache");
        let inbox = root.join("inbox");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::create_dir_all(&inbox).unwrap();
        let name = "0123456789abcdef0123456789abcdef.export";
        let external = root.join(name);
        let received = inbox.join(name);
        let invalid = cache.join("user.export");
        let valid = cache.join(name);
        for path in [&external, &received, &invalid, &valid] {
            std::fs::write(path, b"preserve").unwrap();
        }
        for path in [&external, &received, &invalid] {
            assert!(release_private_file(&cache, path, true).is_err());
            assert_eq!(std::fs::read(path).unwrap(), b"preserve");
        }
        let nested = cache.join("nested");
        std::fs::create_dir(&nested).unwrap();
        let nested_file = nested.join(name);
        std::fs::write(&nested_file, b"nested").unwrap();
        assert!(release_private_file(&cache, &nested_file, true).is_err());
        assert!(nested_file.exists());
        assert!(release_private_file(&cache, &valid, true).is_ok());
        assert!(!valid.exists());
        assert!(release_private_file(&cache, &valid, true).is_ok());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unregistered_selection_release_never_deletes_user_file() {
        let path =
            std::env::temp_dir().join(format!("hermod-user-{}", crate::app::storage::new_id()));
        std::fs::write(&path, b"user data").unwrap();
        release_windows_source(path.to_string_lossy().into_owned()).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"user data");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn source_release_requires_registered_id_and_exact_private_parent() {
        let root =
            std::env::temp_dir().join(format!("hermod-owned-{}", crate::app::storage::new_id()));
        let cache = root.join("authorized-sources");
        std::fs::create_dir_all(&cache).unwrap();
        let owned_path = cache.join("random-snapshot");
        let user_path = root.join("user-original");
        std::fs::write(&owned_path, b"snapshot").unwrap();
        std::fs::write(&user_path, b"original").unwrap();
        let owned_id = crate::app::storage::new_id();
        let invalid_id = crate::app::storage::new_id();
        {
            let mut owned = OWNED_SOURCES.lock().unwrap();
            owned.insert(owned_id.clone(), (cache.clone(), owned_path.clone()));
            owned.insert(invalid_id.clone(), (cache, user_path.clone()));
        }
        assert!(release_windows_source(invalid_id.clone()).is_err());
        assert_eq!(std::fs::read(&user_path).unwrap(), b"original");
        release_windows_source(owned_id.clone()).unwrap();
        release_windows_source(owned_id).unwrap();
        assert!(!owned_path.exists());
        OWNED_SOURCES.lock().unwrap().remove(&invalid_id);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn export_scratch_cleans_on_unwind() {
        let path =
            std::env::temp_dir().join(format!("hermod-export-{}", crate::app::storage::new_id()));
        std::fs::write(&path, b"partial").unwrap();
        let result = std::panic::catch_unwind(|| {
            let _scratch = ExportScratch {
                path: path.clone(),
                keep: false,
            };
            panic!("broker failed");
        });
        assert!(result.is_err());
        assert!(!path.exists());
    }
}
