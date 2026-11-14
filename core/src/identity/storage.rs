//! 私有配置存储：短事务锁、权限验证和原子替换。
//!
//! 路径检查与打开并非 dirfd 相对操作：私有目录阻止其它 UID 修改内部文件，
//! 但调用方仍须提供可信配置目录，不能声称抵御可写祖先被并发替换的所有竞态。
use serde::{Serialize, de::DeserializeOwned};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("storage I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("storage {operation} failed at {path}: {source}")]
    IoAt {
        operation: &'static str,
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid persistent data: {0}")]
    Invalid(&'static str),
    #[error("private storage permissions are not secure")]
    Permissions,
    #[error("private storage validation failed: {0}")]
    PermissionCheck(&'static str),
    #[error("contact not found")]
    NotFound,
    #[error("contact name is ambiguous")]
    Ambiguous,
    #[error("pairing conflicts with existing state")]
    Conflict,
    #[error("device already sending/waiting")]
    Busy,
}
pub type Result<T> = std::result::Result<T, Error>;
fn io_at(operation: &'static str, path: &Path, source: std::io::Error) -> Error {
    Error::IoAt {
        operation,
        path: path.to_path_buf(),
        source,
    }
}

pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(output, "{byte:02x}");
    }
    output
}
pub fn decode_hex<const N: usize>(text: &str) -> Result<[u8; N]> {
    if text.len() != N * 2
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Error::Invalid("noncanonical hex encoding"));
    }
    let mut bytes = [0; N];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16)
            .map_err(|_| Error::Invalid("invalid hex"))?;
    }
    Ok(bytes)
}
pub fn parse_json<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    // 不把 serde 错误文本带出，以免错误包含秘密字段值。
    serde_json::from_slice(bytes).map_err(|_| Error::Invalid("invalid JSON schema"))
}
fn reject_links(path: &Path) -> Result<()> {
    let original = path;
    let path = std::path::absolute(path)?;
    #[cfg(windows)]
    let trusted_root = super::permissions::backend()?
        .private_storage_root()
        .map_err(|error| io_at("resolve private storage boundary", &path, error))?;
    #[cfg(not(windows))]
    let trusted_root: Option<std::path::PathBuf> = None;
    if let Some(root) = &trusted_root {
        if original
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return Err(Error::PermissionCheck(
                "parent traversal in private storage path",
            ));
        }
        validate_bounded_path(&path, root)?;
    }
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    return Err(Error::PermissionCheck("symbolic link in storage path"));
                }
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if meta.file_attributes() & 0x400 != 0 {
                        return Err(Error::PermissionCheck("reparse point in storage path"));
                    }
                }
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && trusted_root.as_deref() != Some(ancestor) => {}
            Err(error) => return Err(io_at("inspect path ancestor", ancestor, error)),
        }
        // Include the system-authorized root itself in reparse checks, but do
        // not probe inaccessible system/user ancestors outside AppContainer.
        if trusted_root.as_deref() == Some(ancestor) {
            break;
        }
    }
    Ok(())
}
fn validate_bounded_path(path: &Path, root: &Path) -> Result<()> {
    use std::path::Component;
    if !root.is_absolute()
        || !path.is_absolute()
        || !path.starts_with(root)
        || path
            .components()
            .chain(root.components())
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return Err(Error::PermissionCheck(
            "path outside system private storage root",
        ));
    }
    Ok(())
}
pub fn prepare_dir(dir: &Path) -> Result<()> {
    let dir = std::path::absolute(dir)?;
    reject_links(&dir)?;
    match fs::symlink_metadata(&dir) {
        Ok(meta) if !meta.is_dir() => return Err(Error::Permissions),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // 只递归创建缺失目录；每层立即收紧，失败不继续。
            if let Some(parent) = dir.parent()
                && !parent.exists()
            {
                prepare_dir(parent)?;
            }
            #[cfg(unix)]
            let mut builder = fs::DirBuilder::new();
            #[cfg(not(unix))]
            let builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&dir) {
                Ok(()) => secure_new(&dir, true)?,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(io_at("create private directory", &dir, error)),
            }
        }
        Err(error) => return Err(io_at("inspect private directory", &dir, error)),
    }
    check_permissions(&dir, true)
}
fn secure_new(path: &Path, directory: bool) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            path,
            fs::Permissions::from_mode(if directory { 0o700 } else { 0o600 }),
        )?;
    }
    #[cfg(windows)]
    {
        super::permissions::backend()?
            .initialize_private(path, directory)
            .map_err(|error| io_at("initialize/verify private ACL", path, error))
    }
    #[cfg(not(windows))]
    check_permissions(path, directory)
}
fn check_permissions(path: &Path, directory: bool) -> Result<()> {
    reject_links(path)?;
    let metadata =
        fs::symlink_metadata(path).map_err(|error| io_at("inspect private path", path, error))?;
    if (directory && !metadata.is_dir()) || (!directory && !metadata.is_file()) {
        return Err(Error::Permissions);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let expected = if directory { 0o700 } else { 0o600 };
        if metadata.permissions().mode() & 0o7777 != expected {
            return Err(Error::Permissions);
        }
        // 不使用可伪造的 HOME 环境变量证明身份。
        let owner = unix_owner()?;
        if metadata.uid() != owner {
            return Err(Error::Permissions);
        }
    }
    #[cfg(windows)]
    super::permissions::backend()?
        .verify_private(path, directory)
        .map_err(|error| io_at("verify private ACL", path, error))?;
    Ok(())
}
#[cfg(unix)]
fn unix_owner() -> Result<u32> {
    // Kernel effective UID, including Android; never infer identity from HOME,
    // /proc ownership (which can change with dumpability), or application paths.
    Ok(rustix::process::geteuid().as_raw())
}
fn create_private(path: &Path) -> Result<File> {
    reject_links(path)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path).map_err(|error| {
        // 保留 AlreadyExists 作为锁文件创建竞争的正常分支。
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            Error::Io(error)
        } else {
            io_at("create private file", path, error)
        }
    })?;
    secure_new(path, false)?;
    Ok(file)
}
pub fn read(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }
    check_permissions(path, false)?;
    let mut file = File::open(path).map_err(|error| io_at("open private file", path, error))?;
    if file.metadata()?.len() > 4 * 1024 * 1024 {
        return Err(Error::Invalid("storage exceeds size limit"));
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(Error::Invalid("storage exceeds size limit"));
    }
    Ok(Some(bytes))
}
#[derive(Debug)]
pub struct Lock {
    _file: File,
}
pub fn lock(dir: &Path, name: &str) -> Result<Lock> {
    open_lock(dir, name, LockMode::Blocking)
}
pub fn try_lock(dir: &Path, name: &str) -> Result<Lock> {
    open_lock(dir, name, LockMode::Nonblocking)
}
enum LockMode {
    Blocking,
    Nonblocking,
}
fn open_lock(dir: &Path, name: &str, mode: LockMode) -> Result<Lock> {
    check_permissions(dir, true)?;
    let path = dir.join(name);
    let file = match create_private(&path) {
        Ok(file) => file,
        Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            check_permissions(&path, false)?;
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .map_err(|error| io_at("open lock file", &path, error))?
        }
        Err(error) => return Err(error),
    };
    match mode {
        LockMode::Blocking => file
            .lock()
            .map_err(|error| io_at("acquire lock", &path, error))?,
        LockMode::Nonblocking => file.try_lock().map_err(|error| match error {
            std::fs::TryLockError::WouldBlock => Error::Busy,
            std::fs::TryLockError::Error(error) => io_at("try acquire lock", &path, error),
        })?,
    }
    Ok(Lock { _file: file })
}
pub fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    if fs::symlink_metadata(path).is_ok() {
        check_permissions(path, false)?;
    }
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|_| Error::Invalid("serialization failed"))?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(Error::Invalid("storage exceeds size limit"));
    }
    let mut random = [0; 16];
    getrandom::fill(&mut random).map_err(|_| Error::Invalid("random source failed"))?;
    let parent = path
        .parent()
        .ok_or(Error::Invalid("missing parent directory"))?;
    let temporary = parent.join(format!(".private-{}.tmp", hex(&random)));
    let mut created = false;
    let result = (|| {
        let mut file = create_private(&temporary)?;
        created = true;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        #[cfg(unix)]
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    // 只删除本次事务创建的路径，绝不清理其它并发事务文件。
    if result.is_err() && created {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_private_boundary_rejects_siblings_and_parent_traversal() {
        let root = std::path::absolute("private-fixture/LocalState").unwrap();
        assert!(validate_bounded_path(&root, &root).is_ok());
        assert!(validate_bounded_path(&root.join("device-state/device.json"), &root).is_ok());
        assert!(
            validate_bounded_path(
                &root.with_file_name("LocalState-other").join("device.json"),
                &root
            )
            .is_err()
        );
        assert!(validate_bounded_path(&root.join("../other/device.json"), &root).is_err());
        assert!(validate_bounded_path(root.parent().unwrap(), &root).is_err());
    }

    #[test]
    fn oversized_atomic_write_preserves_existing_file_and_creates_no_temporary() {
        #[cfg(windows)]
        {
            #[derive(Debug)]
            struct TestPermissions;
            impl crate::identity::StoragePermissions for TestPermissions {
                fn private_storage_root(&self) -> std::io::Result<Option<std::path::PathBuf>> {
                    Ok(None)
                }
                fn initialize_private(&self, _: &Path, _: bool) -> std::io::Result<()> {
                    Ok(())
                }
                fn verify_private(&self, _: &Path, _: bool) -> std::io::Result<()> {
                    Ok(())
                }
            }
            // Explicit non-native test double for transaction/size-limit logic.
            crate::identity::install_storage_permissions(std::sync::Arc::new(TestPermissions))
                .unwrap();
        }
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let dir = std::env::temp_dir().join(format!("hermod-storage-limit-{}", hex(&random)));
        prepare_dir(&dir).unwrap();
        let path = dir.join("contacts.json");
        atomic_json(&path, &"existing").unwrap();
        let previous = fs::read(&path).unwrap();
        let oversized = "x".repeat(4 * 1024 * 1024);
        assert!(matches!(
            atomic_json(&path, &oversized),
            Err(Error::Invalid("storage exceeds size limit"))
        ));
        assert_eq!(fs::read(&path).unwrap(), previous);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        let absent = dir.join("absent.json");
        assert!(atomic_json(&absent, &oversized).is_err());
        assert!(!absent.exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
