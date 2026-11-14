//! Windows broker grants stay as WinRT objects on a dedicated, message-pumping STA.
//!
//! Callers may use these synchronous functions on any FRB worker. The owner HWND
//! must remain alive and its UI thread must keep pumping until the call returns.
//! No external `StorageFile.Path` is read or reopened. Export asks for a directory
//! and atomically creates a NEW basename there; it never replaces an existing file.

use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use windows::{
    Storage::{
        ApplicationData, CreationCollisionOption, FileAccessMode,
        Pickers::{FileOpenPicker, FolderPicker},
        StorageFile, StorageFolder,
        Streams::{DataReader, DataWriter, InputStreamOptions},
    },
    Win32::{
        Foundation::{HWND, WAIT_FAILED},
        System::WinRT::{RO_INIT_SINGLETHREADED, RoInitialize, RoUninitialize},
        UI::{Shell::IInitializeWithWindow, WindowsAndMessaging::*},
    },
    core::{HSTRING, Interface, RuntimeType},
};
use windows_future::{AsyncOperationCompletedHandler, AsyncStatus, IAsyncOperation};

const CHUNK: usize = 64 * 1024;
const DISK_RESERVE: u64 = 64 * 1024 * 1024;
// The lock makes admission/cancel/reset one linearizable operation. Each admitted
// operation owns a distinct flag, so cancellation cannot leak into the next one.
static ACTIVE: Mutex<Option<Arc<AtomicBool>>> = Mutex::new(None);
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct Admission;
impl Drop for Admission {
    fn drop(&mut self) {
        *ACTIVE.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// A private snapshot and its original broker-provided display/transfer basename.
/// The name is metadata only: it must never be used to reopen the external file.
#[derive(Debug, Clone)]
pub struct AuthorizedSource {
    /// Newly created snapshot inside the application's private cache.
    pub path: PathBuf,
    /// Validated original basename, independent of the random cache filename.
    pub name: String,
}

/// Copy a user-selected broker file into a new file in an existing private cache.
/// `cache` must resolve inside the application's system-provided support root.
/// A dismissed picker returns `Ok(None)`; a concurrent picker returns an error.
pub fn pick_to_private(
    window_handle: u64,
    cache: PathBuf,
) -> Result<Option<AuthorizedSource>, String> {
    admitted(move |cancel| {
        let hwnd = owner(window_handle)?;
        let cache = private_directory(&cache)?;
        let picker = at("FileOpenPicker", FileOpenPicker::new())?;
        initialize(&picker, hwnd)?;
        at(
            "FileTypeFilter.Append",
            at("FileTypeFilter", picker.FileTypeFilter())?.Append(&HSTRING::from("*")),
        )?;
        let Some(file) = selected(
            "PickSingleFileAsync",
            at("PickSingleFileAsync start", picker.PickSingleFileAsync())?,
        )?
        else {
            return Ok(None);
        };
        if cancelled(&cancel) {
            return Ok(None);
        }
        // Name is broker metadata, never an authorization path. Preserve it
        // separately from the collision-resistant private scratch basename.
        let name = at("StorageFile.Name", file.Name())?.to_string();
        validate_basename(&name)?;
        let stream = wait(
            "OpenAsync(Read)",
            at(
                "OpenAsync(Read) start",
                file.OpenAsync(FileAccessMode::Read),
            )?,
        )?;
        // Size may be unavailable or the broker file may grow; enforce both an
        // initial full-copy budget (when known) and the reserve before each write.
        let size = stream.Size().ok();
        ensure_disk_budget(&cache, size.unwrap_or(0))?;
        let input = at("GetInputStreamAt", stream.GetInputStreamAt(0))?;
        let reader = at("DataReader", DataReader::CreateDataReader(&input))?;
        at(
            "InputStreamOptions",
            reader.SetInputStreamOptions(InputStreamOptions::Partial),
        )?;
        let (mut target, mut scratch) = new_scratch(&cache)?;
        let copied: Result<(), String> = (|| {
            let mut buffer = [0u8; CHUNK];
            loop {
                if cancelled(&cancel) {
                    return Ok(());
                }
                let load = at("LoadAsync start", reader.LoadAsync(CHUNK as u32))?;
                let op = at("LoadAsync cast", load.cast::<IAsyncOperation<u32>>())?;
                let count = wait("LoadAsync", op)? as usize;
                if count == 0 {
                    break;
                }
                if count > buffer.len() {
                    return Err("broker returned oversized read".into());
                }
                if cancelled(&cancel) {
                    return Ok(());
                }
                ensure_disk_budget(&cache, count as u64)?;
                at("ReadBytes", reader.ReadBytes(&mut buffer[..count]))?;
                target
                    .write_all(&buffer[..count])
                    .map_err(|e| format!("write private cache: {e}"))?;
            }
            target
                .sync_all()
                .map_err(|e| format!("sync private cache: {e}"))?;
            at("DataReader.Close", reader.Close())?;
            at("read stream.Close", stream.Close())?;
            Ok(())
        })();
        // Close the exclusive file handle before the scratch guard may remove it.
        drop(target);
        copied?;
        if cancelled(&cancel) {
            return Ok(None);
        }
        scratch.keep = true;
        Ok(Some(AuthorizedSource {
            path: scratch.path.clone(),
            name,
        }))
    })
}

/// Copy a private source to a NEW file named `name` in a user-selected directory.
/// Existing names are rejected atomically, even empty files. Cancellation returns
/// `Ok(false)`. The private source is never removed (including on success/error).
/// If broker I/O fails, a newly created partial destination may remain; we never
/// perform path-based cleanup on a user directory or delete a broker destination.
pub fn export_from_private(
    window_handle: u64,
    source: PathBuf,
    name: String,
) -> Result<bool, String> {
    validate_basename(&name)?;
    admitted(move |cancel| {
        let hwnd = owner(window_handle)?;
        let root = support_directory_sta()?
            .canonicalize()
            .map_err(|e| format!("resolve private root: {e}"))?;
        let source = source
            .canonicalize()
            .map_err(|e| format!("resolve private source: {e}"))?;
        if !source.starts_with(&root) || source == root {
            return Err("export source must be inside the private support root".into());
        }
        // Sharing reads only prevents writes, rename, and removal while copying.
        let mut input = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&source)
            .map_err(|e| format!("open private source: {e}"))?;
        if !input
            .metadata()
            .map_err(|e| format!("source metadata: {e}"))?
            .is_file()
        {
            return Err("private source is not a regular file".into());
        }
        let picker = at("FolderPicker", FolderPicker::new())?;
        initialize(&picker, hwnd)?;
        at(
            "FolderPicker filter.Append",
            at("FolderPicker filter", picker.FileTypeFilter())?.Append(&HSTRING::from("*")),
        )?;
        let Some(folder): Option<StorageFolder> = selected(
            "PickSingleFolderAsync",
            at(
                "PickSingleFolderAsync start",
                picker.PickSingleFolderAsync(),
            )?,
        )?
        else {
            return Ok(false);
        };
        if cancelled(&cancel) {
            return Ok(false);
        }
        let file: StorageFile = wait(
            "CreateFileAsync(FailIfExists)",
            at(
                "CreateFileAsync start",
                folder.CreateFileAsync(&HSTRING::from(name), CreationCollisionOption::FailIfExists),
            )?,
        )?;
        let stream = wait(
            "OpenAsync(ReadWrite)",
            at(
                "OpenAsync(ReadWrite) start",
                file.OpenAsync(FileAccessMode::ReadWrite),
            )?,
        )?;
        let output = at("GetOutputStreamAt", stream.GetOutputStreamAt(0))?;
        let writer = at("DataWriter", DataWriter::CreateDataWriter(&output))?;
        let mut buffer = [0u8; CHUNK];
        loop {
            if cancelled(&cancel) {
                return Ok(false);
            }
            let count = match input.read(&mut buffer) {
                Ok(count) => count,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(format!("read private source: {e}")),
            };
            if count == 0 {
                break;
            }
            if cancelled(&cancel) {
                return Ok(false);
            }
            at("WriteBytes", writer.WriteBytes(&buffer[..count]))?;
            let store = at("StoreAsync start", writer.StoreAsync())?;
            let op = at("StoreAsync cast", store.cast::<IAsyncOperation<u32>>())?;
            if wait("StoreAsync", op)? as usize != count {
                return Err("broker stream short write; private source retained".into());
            }
        }
        if cancelled(&cancel) {
            return Ok(false);
        }
        if !wait("FlushAsync", at("FlushAsync start", writer.FlushAsync())?)? {
            return Err("broker flush returned false; private source retained".into());
        }
        at("DataWriter.Close", writer.Close())?;
        at("write stream.Close", stream.Close())?;
        Ok(true)
    })
}

/// Return the private system ApplicationData LocalFolder (packaged apps only).
/// An unpackaged developer process receives an explicit error, never an executable
/// directory or an arbitrary environment-variable path masquerading as private.
pub fn support_directory() -> Result<PathBuf, String> {
    on_sta(support_directory_sta)
}

fn support_directory_sta() -> Result<PathBuf, String> {
    let folder = ApplicationData::Current().and_then(|app| app.LocalFolder())
        .map_err(|e| format!("system ApplicationData LocalFolder unavailable (an installed packaged app is required): {e}"))?;
    let path = PathBuf::from(at("ApplicationData.LocalFolder.Path", folder.Path())?.to_string());
    if !path.is_absolute() {
        return Err("system LocalFolder path is not absolute".into());
    }
    Ok(path)
}

fn private_directory(cache: &Path) -> Result<PathBuf, String> {
    let root = support_directory_sta()?
        .canonicalize()
        .map_err(|e| format!("resolve private root: {e}"))?;
    let cache = cache
        .canonicalize()
        .map_err(|e| format!("resolve existing private cache: {e}"))?;
    if !cache.starts_with(&root) || !cache.is_dir() {
        return Err("cache must be an existing directory inside the private support root".into());
    }
    Ok(cache)
}

struct Scratch {
    path: PathBuf,
    parent: PathBuf,
    keep: bool,
}
impl Drop for Scratch {
    fn drop(&mut self) {
        if !self.keep
            && self.path.parent() == Some(self.parent.as_path())
            && self.parent.canonicalize().is_ok_and(|p| p == self.parent)
            && self
                .path
                .symlink_metadata()
                .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
        {
            // Only a create_new file owned by this guard, in its verified private
            // parent. Never recurse or remove broker/user-selected directories.
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn new_scratch(cache: &Path) -> Result<(File, Scratch), String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    for _ in 0..128 {
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = cache.join(format!(
            "hermod-import-{}-{stamp:032x}-{sequence:016x}.tmp",
            std::process::id()
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .share_mode(0)
            .open(&path)
        {
            Ok(file) => {
                return Ok((
                    file,
                    Scratch {
                        path,
                        parent: cache.to_path_buf(),
                        keep: false,
                    },
                ));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("create private scratch: {e}")),
        }
    }
    Err("could not allocate unique private scratch".into())
}

fn validate_basename(name: &str) -> Result<(), String> {
    // No normalization/sanitization: reject rather than silently redirect a name.
    // This layer deliberately has no dependency on the core crate.
    if name.is_empty()
        || name.encode_utf16().count() > 255
        || name.ends_with(['.', ' '])
        || name
            .chars()
            .any(|c| c <= '\u{1f}' || "<>:\"/\\|?*".contains(c))
    {
        return Err("export name must be a legal Windows basename (1..255 UTF-16 units)".into());
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end_matches(' ')
        .to_uppercase();
    let reserved = matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|prefix| {
        stem.strip_prefix(prefix).is_some_and(|suffix| {
            matches!(
                suffix,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    });
    if reserved {
        return Err("export name is a reserved Windows device name".into());
    }
    Ok(())
}

/// Request cancellation of the currently admitted native file operation only.
/// Returns false when idle. A visible system picker is not forcibly dismissed;
/// after it closes, cancellation is observed before copying/creating a file.
/// In-flight broker I/O completes before the next chunk boundary checks the flag.
pub fn cancel_file_operation() -> bool {
    let active = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
    request_cancel(active.as_deref())
}

fn request_cancel(active: Option<&AtomicBool>) -> bool {
    if let Some(flag) = active {
        flag.store(true, Ordering::Release);
        true
    } else {
        false
    }
}

fn cancelled(flag: &AtomicBool) -> bool {
    flag.load(Ordering::Acquire)
}

fn budget_allows(available: u64, pending: u64) -> bool {
    pending
        .checked_add(DISK_RESERVE)
        .is_some_and(|required| available >= required)
}

fn ensure_disk_budget(cache: &Path, pending: u64) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{Win32::Storage::FileSystem::GetDiskFreeSpaceExW, core::PCWSTR};
    let mut wide: Vec<u16> = cache.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err("private cache path contains NUL".into());
    }
    wide.push(0);
    let mut available = 0u64;
    // SAFETY: wide is a live, NUL-terminated UTF-16 path for the duration of the
    // synchronous call; available is a valid exclusive output pointer. Optional
    // unused outputs are NULL. The OS queries only the verified private cache.
    at("GetDiskFreeSpaceExW(private cache)", unsafe {
        GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()), Some(&mut available), None, None)
    })?;
    if !budget_allows(available, pending) {
        return Err(format!(
            "insufficient private cache space: available={available}, pending={pending}, reserve={DISK_RESERVE}"
        ));
    }
    // Advisory preflight only: other processes can consume disk concurrently;
    // every write remains fallible and its scratch is cleaned on failure.
    Ok(())
}

fn admitted<T: Send + 'static>(
    f: impl FnOnce(Arc<AtomicBool>) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut active = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
        if active.is_some() {
            return Err("another broker picker/transfer is already in progress".into());
        }
        *active = Some(cancel.clone());
    }
    let admission = Admission;
    on_sta(move || {
        let _admission = admission;
        f(cancel)
    })
}

fn on_sta<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    std::thread::Builder::new()
        .name("hermod-file-broker-sta".into())
        .spawn(move || {
            // SAFETY: a fresh OS thread has no prior COM initialization. All WinRT
            // objects and the apartment guard live and drop on this same thread.
            at("RoInitialize(STA)", unsafe {
                RoInitialize(RO_INIT_SINGLETHREADED)
            })?;
            let _apartment = Apartment;
            f()
        })
        .map_err(|e| format!("spawn broker STA: {e}"))?
        .join()
        .map_err(|_| "broker STA panicked".to_string())?
}

struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: balances exactly one successful RoInitialize on this thread;
        // the closure's COM objects have already been dropped.
        unsafe { RoUninitialize() };
    }
}

fn owner(value: u64) -> Result<HWND, String> {
    let raw = usize::try_from(value).map_err(|_| "HWND exceeds pointer width".to_string())?;
    if raw == 0 {
        return Err("owner HWND is zero".into());
    }
    let hwnd = HWND(raw as *mut std::ffi::c_void);
    let mut pid = 0;
    // SAFETY: APIs validate an opaque borrowed handle, without dereferencing Rust
    // memory. The runner owns this window; we never destroy or take ownership.
    let valid = unsafe {
        IsWindow(Some(hwnd)).as_bool()
            && GetAncestor(hwnd, GA_ROOT) == hwnd
            && GetWindowThreadProcessId(hwnd, Some(&mut pid)) != 0
    };
    if !valid || pid != std::process::id() {
        return Err("owner must be a live same-process top-level HWND".into());
    }
    Ok(hwnd)
}

fn initialize<T: Interface>(picker: &T, hwnd: HWND) -> Result<(), String> {
    let init: IInitializeWithWindow = at("QI IInitializeWithWindow", picker.cast())?;
    // SAFETY: owner() validated this borrowed same-process top-level HWND. The
    // runner must keep it alive while awaiting; the interface stays on this STA.
    at("IInitializeWithWindow.Initialize", unsafe {
        init.Initialize(hwnd)
    })
}

fn at<T>(stage: &str, result: windows::core::Result<T>) -> Result<T, String> {
    result.map_err(|e| format!("{stage}: {e} (HRESULT 0x{:08X})", e.code().0 as u32))
}

fn pump<T: RuntimeType + 'static>(op: &IAsyncOperation<T>) -> Result<(), String> {
    let done = Arc::new(AtomicBool::new(false));
    let signal = done.clone();
    // Completion may be inline, on this STA, or an agile worker; it touches only
    // an owned atomic and cannot panic or expose COM objects across threads.
    let handler = AsyncOperationCompletedHandler::new(move |_, _| {
        signal.store(true, Ordering::Release);
        Ok(())
    });
    at("SetCompleted", op.SetCompleted(&handler))?;
    while !done.load(Ordering::Acquire) {
        // SAFETY: MSG is initialized and local; no borrowed Rust memory crosses
        // message dispatch. Finite wait observes callbacks not posting messages.
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    let _ = op.Cancel();
                    return Err("broker STA received WM_QUIT".into());
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            if !done.load(Ordering::Acquire)
                && MsgWaitForMultipleObjectsEx(None, 25, QS_ALLINPUT, MWMO_INPUTAVAILABLE)
                    == WAIT_FAILED
            {
                let _ = op.Cancel();
                return Err(format!(
                    "broker message wait: {}",
                    windows::core::Error::from_thread()
                ));
            }
        }
    }
    Ok(())
}

fn wait<T: RuntimeType + 'static>(stage: &str, op: IAsyncOperation<T>) -> Result<T, String> {
    pump(&op)?;
    at(stage, op.GetResults())
}

fn selected<T: RuntimeType + Interface + 'static>(
    stage: &str,
    op: IAsyncOperation<T>,
) -> Result<Option<T>, String> {
    pump(&op)?;
    match op.GetResults() {
        Ok(item) => Ok(Some(item)),
        // A successful NULL interface projects to Error(S_OK). Only accept that
        // with corroborating Completed + S_OK, never swallow real E_POINTER.
        Err(e)
            if e.code().0 == 0
                && at("picker Status", op.Status())? == AsyncStatus::Completed
                && at("picker ErrorCode", op.ErrorCode())?.0 == 0 =>
        {
            Ok(None)
        }
        Err(e) => at(stage, Err(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::{DISK_RESERVE, budget_allows, cancelled, request_cancel, validate_basename};
    use std::sync::atomic::AtomicBool;

    #[test]
    fn disk_budget_preserves_reserve_and_rejects_overflow() {
        assert!(budget_allows(DISK_RESERVE, 0));
        assert!(!budget_allows(DISK_RESERVE - 1, 0));
        assert!(budget_allows(DISK_RESERVE + 65536, 65536));
        assert!(!budget_allows(DISK_RESERVE + 65535, 65536));
        assert!(!budget_allows(u64::MAX, u64::MAX));
    }

    #[test]
    fn cancellation_only_marks_the_current_operation() {
        assert!(!request_cancel(None));
        let current = AtomicBool::new(false);
        let next = AtomicBool::new(false);
        assert!(!cancelled(&current));
        assert!(request_cancel(Some(&current)));
        assert!(cancelled(&current));
        assert!(!cancelled(&next));
    }

    #[test]
    fn rejects_paths_streams_devices_and_ambiguous_names() {
        for name in [
            "",
            ".",
            "..",
            "a/b",
            "a\\b",
            "C:foo",
            "foo:stream",
            "NUL.txt",
            "com1",
            "LPT³.log",
            "CONIN$",
            "x.",
            "x ",
            "a\0b",
            "a\nb",
        ] {
            assert!(validate_basename(name).is_err(), "accepted {name:?}");
        }
        for name in [
            "transfer.bin",
            "会议记录.txt",
            "COM10",
            ".hidden",
            "a b.txt",
        ] {
            assert!(validate_basename(name).is_ok(), "rejected {name:?}");
        }
        assert!(validate_basename(&"x".repeat(256)).is_err());
    }
}
