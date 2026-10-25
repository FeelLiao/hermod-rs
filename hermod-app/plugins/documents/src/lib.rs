//! Android SAF、动态配色和前台服务适配；流通过官方 fs 插件安全打开。
use tauri::{Runtime, plugin::TauriPlugin};
#[cfg(target_os = "android")]
pub struct Documents<R: Runtime>(tauri::plugin::PluginHandle<R>);
#[cfg(target_os = "android")]
impl<R: Runtime> Documents<R> {
    pub fn call<T: serde::de::DeserializeOwned>(
        &self,
        command: &str,
        args: serde_json::Value,
    ) -> Result<T, String> {
        self.0
            .run_mobile_plugin(command, args)
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
