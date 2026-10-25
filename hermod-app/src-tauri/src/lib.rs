//! Tauri 原生命令：权限选择、流式传输和应用私有收件箱。
mod storage;
mod worker;

use hermod_core::{
    config::{self, Config},
    session::SessionOptions,
};
use serde::Serialize;
use std::{path::PathBuf, sync::Mutex};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::{DialogExt, FilePath};
use tauri_plugin_fs::{FsExt, OpenOptions};
use worker::{Action, Job, Source};

struct AppState {
    _storage_lock: std::fs::File,
    inbox: PathBuf,
    cache: PathBuf,
    config: Config,
    job: Mutex<Option<Job>>,
    source: Mutex<Option<(String, Source)>>,
    // 覆盖选择器、导出与启动的异步全过程，避免重复点击形成并发任务。
    operation: tokio::sync::Mutex<()>,
}

#[derive(Serialize)]
struct Selection {
    id: String,
    name: String,
    size: Option<u64>,
}
#[derive(Serialize)]
struct Bootstrap {
    version: String,
    server: String,
    ipv6: Vec<String>,
    platform: String,
}
#[derive(Serialize)]
struct Status {
    phase: String,
    code: Option<String>,
    bytes: u64,
    total: Option<u64>,
    elapsed: f64,
    done: bool,
    cancelled: bool,
    error: Option<String>,
    digest: Option<String>,
    rtt_ms: Option<f64>,
}

fn ensure_idle(state: &AppState) -> Result<(), String> {
    if state
        .job
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|j| j.snapshot().completed.is_none())
    {
        return Err("A transfer is already running".into());
    }
    Ok(())
}

#[tauri::command]
fn bootstrap(state: State<'_, AppState>) -> Bootstrap {
    let ipv6 = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|i| match i.ip() {
            std::net::IpAddr::V6(ip) if !ip.is_loopback() && !ip.is_unicast_link_local() => {
                Some(ip.to_string())
            }
            _ => None,
        })
        .collect();
    Bootstrap {
        version: env!("CARGO_PKG_VERSION").into(),
        server: state.config.server_url.clone(),
        ipv6,
        platform: std::env::consts::OS.into(),
    }
}

#[tauri::command]
async fn choose_source(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<Selection>, String> {
    let _guard = state
        .operation
        .try_lock()
        .map_err(|_| "Another file operation is in progress")?;
    ensure_idle(&state)?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Choose a file to send")
        .pick_file(move |file| {
            let _ = tx.send(file);
        });
    let Some(path) = rx.await.map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let cache = state.cache.join(storage::new_id());
    let source = tauri::async_runtime::spawn_blocking(move || -> Result<Source, String> {
        let mut options = OpenOptions::new();
        options.read(true);
        let file = app
            .fs()
            .open(path.clone(), options)
            .map_err(|e| e.to_string())?;
        let (name, size) = match path {
            FilePath::Path(path) => {
                let metadata = file.metadata().map_err(|e| e.to_string())?;
                if !metadata.is_file() {
                    return Err("Select a regular file".into());
                }
                (
                    path.file_name()
                        .ok_or("Invalid filename")?
                        .to_string_lossy()
                        .into_owned(),
                    Some(metadata.len()),
                )
            }
            FilePath::Url(_uri) => {
                #[cfg(target_os = "android")]
                {
                    let info = app
                        .state::<tauri_plugin_documents::Documents<tauri::Wry>>()
                        .describe(_uri.as_str())?;
                    (info.name, info.size)
                }
                #[cfg(not(target_os = "android"))]
                {
                    return Err("Unsupported document URL".into());
                }
            }
        };
        // 提前验证目标名称；接收端统一执行跨平台名称规范化。
        hermod_core::transfer::safe_destination_path(&std::env::temp_dir(), &name)
            .map_err(|e| e.to_string())?;
        Ok(Source {
            file,
            name,
            size,
            cache,
        })
    })
    .await
    .map_err(|e| e.to_string())??;
    let id = storage::new_id();
    let selection = Selection {
        id: id.clone(),
        name: source.name.clone(),
        size: source.size,
    };
    *state.source.lock().unwrap() = Some((id, source));
    Ok(Some(selection))
}

fn set_active(_app: &AppHandle, _active: bool) -> Result<(), String> {
    #[cfg(target_os = "android")]
    _app.state::<tauri_plugin_documents::Documents<tauri::Wry>>()
        .set_active(_active)?;
    Ok(())
}

#[tauri::command]
async fn start_transfer(
    app: AppHandle,
    state: State<'_, AppState>,
    source_id: Option<String>,
    code: Option<String>,
    limit_mbps: String,
) -> Result<(), String> {
    let _guard = state
        .operation
        .try_lock()
        .map_err(|_| "Another file operation is in progress")?;
    ensure_idle(&state)?;
    let limit = limit_mbps.parse().map_err(|e: &str| e.to_string())?;
    let action = match (source_id, code) {
        (Some(id), None) => {
            let mut slot = state.source.lock().unwrap();
            if slot.as_ref().is_none_or(|(selected, _)| selected != &id) {
                return Err("Choose the file again".into());
            }
            set_active(&app, true)?;
            Action::Send(slot.take().unwrap().1)
        }
        (None, Some(code)) => {
            let code = code.trim().to_string();
            if code.len() > 200 || !code.contains('-') || code.chars().any(char::is_whitespace) {
                return Err("Enter a valid transfer code".into());
            }
            let directory = state.inbox.join(storage::new_id()).join("payload");
            tokio::fs::create_dir_all(&directory)
                .await
                .map_err(|e| e.to_string())?;
            set_active(&app, true)?;
            Action::Receive { code, directory }
        }
        _ => return Err("Choose either a file or a transfer code".into()),
    };
    let job = Job::start(
        state.config.clone(),
        SessionOptions {
            bandwidth_limit: limit,
            ..Default::default()
        },
        action,
        Box::new(move || {
            let _ = set_active(&app, false);
        }),
    );
    *state.job.lock().unwrap() = Some(job);
    Ok(())
}

#[tauri::command]
fn status(state: State<'_, AppState>) -> Option<Status> {
    state.job.lock().unwrap().as_ref().map(|job| {
        let s = job.snapshot();
        Status {
            elapsed: s
                .completed
                .unwrap_or_else(std::time::Instant::now)
                .duration_since(s.started)
                .as_secs_f64(),
            done: s.completed.is_some(),
            phase: s.phase,
            code: s.code,
            bytes: s.bytes,
            total: s.total,
            cancelled: s.cancelled,
            error: s.error,
            digest: s.digest,
            rtt_ms: s.network.map(|n| n.snapshot().rtt.as_secs_f64() * 1000.0),
        }
    })
}

#[tauri::command]
fn cancel_transfer(state: State<'_, AppState>) {
    if let Some(job) = state.job.lock().unwrap().as_ref() {
        job.cancel();
    }
}

#[tauri::command]
fn copy_code(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    let code = state
        .job
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|j| j.snapshot().code)
        .ok_or("No transfer code yet")?;
    app.clipboard().write_text(code).map_err(|e| e.to_string())
}

#[tauri::command]
async fn list_received(state: State<'_, AppState>) -> Result<Vec<storage::Received>, String> {
    let root = state.inbox.clone();
    tauri::async_runtime::spawn_blocking(move || storage::list(&root))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn export_received(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<bool, String> {
    let _guard = state
        .operation
        .try_lock()
        .map_err(|_| "Another file operation is in progress")?;
    ensure_idle(&state)?;
    let (item, source) = storage::resolve(&state.inbox, &id)?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Save verified file")
        .set_file_name(&item.name)
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    let Some(destination) = rx.await.map_err(|e| e.to_string())? else {
        return Ok(false);
    };
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        if let FilePath::Path(path) = &destination
            && path.canonicalize().is_ok_and(|p| p == source)
        {
            return Err("Choose a location outside the private inbox".into());
        }
        let mut input = std::fs::File::open(source).map_err(|e| e.to_string())?;
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        let mut output = app
            .fs()
            .open(destination, options)
            .map_err(|e| e.to_string())?;
        let copied = std::io::copy(&mut input, &mut output).map_err(|e| e.to_string())?;
        if copied != item.size {
            return Err("Export size mismatch; verified inbox copy retained".into());
        }
        // 云端文档提供器可以返回管道，不能对管道调用 fsync。
        if output.metadata().map_err(|e| e.to_string())?.is_file() {
            output.sync_all().map_err(|e| e.to_string())?;
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(true)
}

#[tauri::command]
async fn delete_received(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let _guard = state
        .operation
        .try_lock()
        .map_err(|_| "Another file operation is in progress")?;
    ensure_idle(&state)?;
    let root = state.inbox.clone();
    tauri::async_runtime::spawn_blocking(move || storage::remove(&root, &id))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(not(target_os = "android"))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _, _| {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.set_focus();
        }
    }));
    builder
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_documents::init())
        .setup(|app| {
            let data = app.path().app_data_dir()?;
            let cache = app.path().app_cache_dir()?.join("outgoing");
            std::fs::create_dir_all(&data)?;
            let storage_lock = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(data.join("storage.lock"))?;
            storage_lock.try_lock().map_err(std::io::Error::other)?;
            let inbox = data.join("received");
            std::fs::create_dir_all(&inbox)?;
            storage::clean_incomplete(&inbox).map_err(std::io::Error::other)?;
            // 进程已退出时留下的未知长度缓存可以安全回收。
            let _ = std::fs::remove_dir_all(&cache);
            std::fs::create_dir_all(&cache)?;
            let config_path = app.path().app_config_dir()?.join("config.yaml");
            let config = if config_path.exists() {
                config::load_from(&config_path)?
            } else {
                #[cfg(not(target_os = "android"))]
                let config = config::load()?;
                #[cfg(target_os = "android")]
                let config = Config::default();
                config
            };
            app.manage(AppState {
                _storage_lock: storage_lock,
                inbox,
                cache,
                config,
                job: Mutex::new(None),
                source: Mutex::new(None),
                operation: tokio::sync::Mutex::new(()),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            choose_source,
            start_transfer,
            status,
            cancel_transfer,
            copy_code,
            list_received,
            export_received,
            delete_received
        ])
        .run(tauri::generate_context!())
        .expect("Unable to start Hermod");
}
