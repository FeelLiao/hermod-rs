//! Android 文档元数据和传输前台服务；文件流由官方 fs 插件打开。
use tauri::{Runtime, plugin::TauriPlugin};

#[derive(serde::Deserialize)]
pub struct DocumentInfo {
    pub name: String,
    pub size: Option<u64>,
}

#[cfg(target_os = "android")]
pub struct Documents<R: Runtime>(tauri::plugin::PluginHandle<R>);

#[cfg(target_os = "android")]
impl<R: Runtime> Documents<R> {
    pub fn describe(&self, uri: &str) -> Result<DocumentInfo, String> {
        self.0
            .run_mobile_plugin("describe", serde_json::json!({ "uri": uri }))
            .map_err(|e| e.to_string())
    }
    pub fn set_active(&self, active: bool) -> Result<(), String> {
        self.0
            .run_mobile_plugin::<serde_json::Value>(
                "setActive",
                serde_json::json!({ "active": active }),
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    tauri::plugin::Builder::new("documents")
        .setup(|_app, _api| {
            #[cfg(target_os = "android")]
            {
                use tauri::Manager;
                let handle =
                    _api.register_android_plugin("org.hermod.documents", "DocumentsPlugin")?;
                _app.manage(Documents(handle));
            }
            Ok(())
        })
        .build()
}
