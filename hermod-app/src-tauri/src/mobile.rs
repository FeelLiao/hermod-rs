//! Android 平台边界：源文件 URI 只留在 Rust，数据流不经过 WebView。
use crate::{
    AppState,
    app::{Bootstrap, Selection},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use tauri::{AppHandle, Manager};
use tauri_plugin_fs::{FilePath, FsExt, OpenOptions};

#[derive(Deserialize)]
struct Document {
    uri: Option<String>,
    name: Option<String>,
    size: Option<u64>,
}
fn call<T: serde::de::DeserializeOwned>(
    app: &AppHandle,
    method: &str,
    args: Value,
) -> Result<T, String> {
    app.state::<tauri_plugin_documents::Documents<tauri::Wry>>()
        .call(method, args)
}
pub fn palette(app: &AppHandle, info: &mut Bootstrap) {
    if let Ok(value) = call::<Value>(app, "palette", json!({})) {
        info.color_seed = value["seed"].as_str().map(str::to_owned);
    }
}
pub fn choose(app: &AppHandle, engine: &crate::app::Engine) -> Result<Option<Selection>, String> {
    let doc: Document = call(app, "pickSource", json!({}))?;
    let Some(uri) = doc.uri else {
        return Ok(None);
    };
    let path: FilePath = uri
        .parse()
        .map_err(|e| format!("Invalid document URI: {e}"))?;
    let mut options = OpenOptions::new();
    options.read(true);
    let file = app.fs().open(path, options).map_err(|e| e.to_string())?;
    engine
        .select(
            Box::new(file),
            doc.name.unwrap_or_else(|| "file".into()),
            doc.size,
        )
        .map(Some)
}
pub fn folder(app: &AppHandle) -> Result<Option<String>, String> {
    let doc: Document = call(app, "pickFolder", json!({}))?;
    Ok(doc.uri)
}
pub fn prepare(app: &AppHandle) -> Result<(), String> {
    call::<Value>(app, "prepare", json!({})).map(|_| ())
}
pub fn active(app: &AppHandle, value: bool) -> Result<(), String> {
    call::<Value>(app, "setActive", json!({"active": value})).map(|_| ())
}
pub fn progress(app: &AppHandle, status: &crate::app::Status) -> Result<bool, String> {
    call::<Value>(
        app,
        "progress",
        json!({"phase":status.phase,"bytes":status.bytes,"total":status.total}),
    )
    .map(|v| v["cancel"].as_bool().unwrap_or(false))
}
pub fn export(
    app: &AppHandle,
    engine: &Arc<crate::app::Engine>,
    id: &str,
    folder: Option<String>,
) -> Result<bool, String> {
    engine.ensure_idle()?;
    let (item, _) = crate::app::storage::resolve(&engine.inbox, id)?;
    let doc: Document = call(
        app,
        "createExport",
        json!({"name":item.name,"folder":folder}),
    )?;
    let Some(uri) = doc.uri else {
        return Ok(false);
    };
    let result = (|| {
        let path: FilePath = uri
            .parse()
            .map_err(|e| format!("Invalid document URI: {e}"))?;
        let mut options = OpenOptions::new();
        options.write(true).truncate(true);
        let mut file = app.fs().open(path, options).map_err(|e| e.to_string())?;
        engine.export(id, &mut file)
    })();
    if result.is_err() {
        // 仅清除这次新建的导出文档；验证过的私有副本始终保留。
        let _ = call::<Value>(app, "removePartial", json!({"uri":uri}));
    }
    result.map(|_| true)
}
pub async fn start(
    app: AppHandle,
    source_id: Option<String>,
    code: Option<String>,
    text: Option<String>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        prepare(&app)?;
        active(&app, true)?;
        if let Err(error) = state.engine.start(source_id, code, text) {
            let _ = active(&app, false);
            return Err(error);
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}
