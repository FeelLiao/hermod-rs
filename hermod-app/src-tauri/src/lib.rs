//! Tauri 平台适配层；传输状态保存在 Rust，Web 页面只操作授权 ID。
pub mod app;
mod i18n;
#[cfg(target_os = "android")]
mod mobile;
use app::{Bootstrap, Engine, Selection, Settings, Status, storage};
#[cfg(not(target_os = "android"))]
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use tauri::{AppHandle, Manager, State};
#[cfg(not(target_os = "android"))]
use tauri_plugin_dialog::{DialogExt, FilePath};
struct AppState {
    locale: Mutex<String>,
    engine: Arc<Engine>,
    operation: tokio::sync::Mutex<()>,
    monitoring: AtomicBool,
    export_error: Mutex<Option<String>>,
}
impl AppState {
    fn text(&self, message: &str) -> String {
        i18n::translate(&self.locale.lock().unwrap(), message).to_owned()
    }
}
#[tauri::command]
fn set_locale(state: State<'_, AppState>, locale: String) -> Result<(), String> {
    if !["en-US", "zh-CN"].contains(&locale.as_str()) {
        return Err("Invalid language".into());
    }
    *state.locale.lock().unwrap() = locale;
    Ok(())
}
#[tauri::command]
async fn bootstrap(app: AppHandle, state: State<'_, AppState>) -> Result<Bootstrap, String> {
    let info = state.engine.bootstrap();
    #[cfg(target_os = "android")]
    return tauri::async_runtime::spawn_blocking(move || {
        let mut info = info;
        mobile::palette(&app, &mut info);
        info
    })
    .await
    .map_err(|e| e.to_string());
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Ok(info)
    }
}
#[tauri::command]
fn save_settings(state: State<'_, AppState>, settings: Settings) -> Result<(), String> {
    state.engine.save_settings(settings)
}
#[tauri::command]
fn status(state: State<'_, AppState>) -> Option<Status> {
    state.engine.status().map(|mut status| {
        status.export_error = state.export_error.lock().unwrap().clone();
        status
    })
}
#[tauri::command]
fn cancel_transfer(state: State<'_, AppState>) {
    state.engine.cancel();
}
#[tauri::command]
fn get_logs(state: State<'_, AppState>) -> Vec<String> {
    state.engine.logs()
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
    state.engine.ensure_idle()?;
    #[cfg(target_os = "android")]
    {
        let engine = state.engine.clone();
        return tauri::async_runtime::spawn_blocking(move || mobile::choose(&app, &engine))
            .await
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(target_os = "android"))]
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.dialog()
            .file()
            .set_title(state.text("Choose a file to send"))
            .pick_file(move |value| {
                let _ = tx.send(value);
            });
        match rx.await.map_err(|e| e.to_string())? {
            Some(FilePath::Path(path)) => state.engine.select_file(&path).map(Some),
            None => Ok(None),
            _ => Err("Choose a local file".into()),
        }
    }
}
#[tauri::command]
async fn choose_destination(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let _guard = state
        .operation
        .try_lock()
        .map_err(|_| "Another file operation is in progress")?;
    #[cfg(target_os = "android")]
    let destination = tauri::async_runtime::spawn_blocking(move || mobile::folder(&app))
        .await
        .map_err(|e| e.to_string())??;
    #[cfg(not(target_os = "android"))]
    let destination = {
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.dialog()
            .file()
            .set_title(state.text("Default export folder"))
            .pick_folder(move |value| {
                let _ = tx.send(value);
            });
        match rx.await.map_err(|e| e.to_string())? {
            Some(FilePath::Path(path)) => Some(path.to_string_lossy().into_owned()),
            _ => None,
        }
    };
    if let Some(value) = &destination {
        state.engine.set_destination(Some(value.clone()))?;
    }
    Ok(destination)
}
#[tauri::command]
fn clear_destination(state: State<'_, AppState>) -> Result<(), String> {
    state.engine.set_destination(None)
}
#[tauri::command]
async fn start_transfer(
    app: AppHandle,
    state: State<'_, AppState>,
    source_id: Option<String>,
    code: Option<String>,
    text: Option<String>,
) -> Result<(), String> {
    let _guard = state
        .operation
        .try_lock()
        .map_err(|_| "Another file operation is in progress")?;
    if state.monitoring.load(Ordering::Acquire) {
        return Err("Finishing the previous transfer; please try again".into());
    }
    #[cfg(target_os = "android")]
    mobile::start(app.clone(), source_id, code, text).await?;
    #[cfg(not(target_os = "android"))]
    state.engine.start(source_id, code, text)?;
    *state.export_error.lock().unwrap() = None;
    state.monitoring.store(true, Ordering::Release);
    monitor(app);
    Ok(())
}
#[tauri::command]
fn copy_code(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    let code = state
        .engine
        .status()
        .and_then(|s| s.code)
        .ok_or("No transfer code yet")?;
    app.clipboard().write_text(code).map_err(|e| e.to_string())
}
#[tauri::command]
fn list_received(state: State<'_, AppState>) -> Result<Vec<storage::Received>, String> {
    state.engine.list_received()
}
#[tauri::command]
async fn export_received(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    use_default: bool,
) -> Result<bool, String> {
    let _guard = state
        .operation
        .try_lock()
        .map_err(|_| "Another file operation is in progress")?;
    state.engine.ensure_idle()?;
    #[cfg(target_os = "android")]
    {
        let folder = if use_default {
            let Some(folder) = state.engine.transfer_destination() else {
                return Ok(false);
            };
            Some(folder)
        } else {
            None
        };
        let engine = state.engine.clone();
        return tauri::async_runtime::spawn_blocking(move || {
            mobile::export(&app, &engine, &id, folder)
        })
        .await
        .map_err(|e| e.to_string())?;
    }
    #[cfg(not(target_os = "android"))]
    {
        let (item, _) = storage::resolve(&state.engine.inbox, &id)?;
        let destination = if use_default {
            let Some(folder) = state.engine.transfer_destination() else {
                return Ok(false);
            };
            // 自动导出必须使用与接收相同的不覆盖路径策略。
            Some(
                hermod_core::transfer::safe_destination_path(&PathBuf::from(folder), &item.name)
                    .map_err(|e| e.to_string())?,
            )
        } else {
            let (tx, rx) = tokio::sync::oneshot::channel();
            app.dialog()
                .file()
                .set_title(state.text("Save verified file"))
                .set_file_name(&item.name)
                .save_file(move |value| {
                    let _ = tx.send(value);
                });
            match rx.await.map_err(|e| e.to_string())? {
                Some(FilePath::Path(path)) => Some(path),
                None => None,
                _ => return Err("Choose a local path".into()),
            }
        };
        let Some(path) = destination else {
            return Ok(false);
        };
        let engine = state.engine.clone();
        tauri::async_runtime::spawn_blocking(move || engine.export_path(&id, &path, !use_default))
            .await
            .map_err(|e| e.to_string())??;
        Ok(true)
    }
}
#[tauri::command]
fn delete_received(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.engine.remove(&id)
}
#[tauri::command]
async fn probe_server(state: State<'_, AppState>, server: String) -> Result<String, String> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || engine.probe_server(&server))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn trust_server(
    state: State<'_, AppState>,
    server: String,
    fingerprint: String,
) -> Result<(), String> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || engine.trust_server(&server, &fingerprint))
        .await
        .map_err(|e| e.to_string())?
}
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(target_os = "android")]
    let builder = builder
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_documents::init());
    #[cfg(not(target_os = "android"))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _, _| {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.set_focus();
        }
    }));
    builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            let legacy = app.path().app_config_dir()?.join("config.yaml");
            let fallback = if legacy.exists() {
                hermod_core::config::load_from(&legacy)?
            } else {
                #[cfg(not(target_os = "android"))]
                let fallback = hermod_core::config::load()?;
                #[cfg(target_os = "android")]
                let fallback = hermod_core::config::Config::default();
                fallback
            };
            let engine =
                Engine::new(app.path().app_data_dir()?, fallback).map_err(std::io::Error::other)?;
            app.manage(AppState {
                locale: Mutex::new("en-US".into()),
                engine: Arc::new(engine),
                operation: tokio::sync::Mutex::new(()),
                monitoring: AtomicBool::new(false),
                export_error: Mutex::new(None),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            set_locale,
            bootstrap,
            save_settings,
            status,
            cancel_transfer,
            get_logs,
            choose_source,
            choose_destination,
            clear_destination,
            start_transfer,
            copy_code,
            list_received,
            export_received,
            delete_received,
            probe_server,
            trust_server
        ])
        .run(tauri::generate_context!())
        .expect("Unable to start Hermod");
}

// 后台任务监督不依赖 WebView 定时器；完成后自动导出并结束前台服务。
fn monitor(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let Some(status) = state.engine.status() else {
                break;
            };
            if status.done {
                let _guard = state.operation.lock().await;
                if let (Some(id), Some(folder)) =
                    (status.received_id, state.engine.transfer_destination())
                {
                    let engine = state.engine.clone();
                    #[cfg(target_os = "android")]
                    let handle = app.clone();
                    let result = tauri::async_runtime::spawn_blocking(move || {
                        #[cfg(target_os = "android")]
                        {
                            mobile::export(&handle, &engine, &id, Some(folder)).map(|_| ())
                        }
                        #[cfg(not(target_os = "android"))]
                        {
                            let (item, _) = storage::resolve(&engine.inbox, &id)?;
                            let path = hermod_core::transfer::safe_destination_path(
                                &PathBuf::from(folder),
                                &item.name,
                            )
                            .map_err(|e| e.to_string())?;
                            engine.export_path(&id, &path, false)
                        }
                    })
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|r| r);
                    if let Err(error) = result {
                        let message = format!(
                            "Export failed: {error}. Verified inbox copy retained; use Save a copy to retry."
                        );
                        tracing::error!("{message}");
                        *state.export_error.lock().unwrap() = Some(message);
                    }
                }
                break;
            }
            #[cfg(target_os = "android")]
            {
                let handle = app.clone();
                match tauri::async_runtime::spawn_blocking(move || {
                    mobile::progress(&handle, &status)
                })
                .await
                {
                    Ok(Ok(false)) => {}
                    _ => state.engine.cancel(),
                }
            }
        }
        #[cfg(target_os = "android")]
        {
            let handle = app.clone();
            let _ =
                tauri::async_runtime::spawn_blocking(move || mobile::active(&handle, false)).await;
        }
        state.monitoring.store(false, Ordering::Release);
    });
}
