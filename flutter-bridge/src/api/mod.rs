//! flutter_rust_bridge 暴露的 GUI 能力面。
//!
//! 只转发 [`crate::gui_api::GuiController`]；传输、信任、私有收件箱路径
//! 仍由 Engine 管理，此处仅限类型化 DTO 与不透明 id。
use hermod_core::config::Config;
use hermod_core::options::{Congestion, ConnectionSettings, Mtu};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
pub mod events;
pub mod platform;

use crate::gui_api::{
    BootstrapDto, GuiError, ReceivedDto, SelectionDto, StartRequest, TransferSnapshot,
};

#[flutter_rust_bridge::frb(non_opaque)]
#[derive(Debug, Clone)]
pub struct ApiConnection {
    pub transport: String,
    pub mtu: String,
    pub congestion: String,
    /// 链路档位：auto / lan / wan。
    pub profile: String,
}

#[flutter_rust_bridge::frb(non_opaque)]
#[derive(Debug, Clone)]
pub struct ApiTuning {
    pub streams: Option<u32>,
    pub chunk_bytes: u32,
    pub window_bytes: u64,
    pub udp_buffer_bytes: u32,
    pub slice_bytes: u64,
    pub prefetch: u32,
    pub keepalive_secs: u32,
    pub idle_timeout_secs: u32,
}
impl From<crate::app::tuning::TuningSettings> for ApiTuning {
    fn from(t: crate::app::tuning::TuningSettings) -> Self {
        Self {
            streams: t.streams,
            chunk_bytes: t.chunk_bytes,
            window_bytes: t.window_bytes,
            udp_buffer_bytes: t.udp_buffer_bytes,
            slice_bytes: t.slice_bytes,
            prefetch: t.prefetch,
            keepalive_secs: t.keepalive_secs,
            idle_timeout_secs: t.idle_timeout_secs,
        }
    }
}
impl From<ApiTuning> for crate::app::tuning::TuningSettings {
    fn from(t: ApiTuning) -> Self {
        Self {
            streams: t.streams,
            chunk_bytes: t.chunk_bytes,
            window_bytes: t.window_bytes,
            udp_buffer_bytes: t.udp_buffer_bytes,
            slice_bytes: t.slice_bytes,
            prefetch: t.prefetch,
            keepalive_secs: t.keepalive_secs,
            idle_timeout_secs: t.idle_timeout_secs,
        }
    }
}

#[flutter_rust_bridge::frb(non_opaque)]
#[derive(Debug, Clone)]
pub struct ApiSettings {
    pub version: u32,
    pub connection: ApiConnection,
    pub tuning: ApiTuning,
    pub receive_transport: String,
    /// 地址族约束：auto / ipv4 / ipv6。
    pub ip_family: String,
    pub server: String,
    pub words: u32,
    pub verbose: String,
    pub quiet: bool,
    pub theme: String,
    pub language: String,
    pub destination: Option<String>,
}

#[flutter_rust_bridge::frb(non_opaque)]
#[derive(Debug, Clone)]
pub struct ApiBootstrap {
    pub device_id: String,
    pub version: String,
    pub ipv4: Vec<String>,
    pub ipv6: Vec<String>,
    pub platform: String,
    pub settings_saved: bool,
    pub settings: ApiSettings,
}

impl ApiConnection {
    fn from_engine(connection: &ConnectionSettings) -> Self {
        Self {
            transport: connection.transport.clone(),
            mtu: connection.mtu.to_string(),
            congestion: match connection.congestion {
                Congestion::Auto => "auto",
                Congestion::Bbr => "bbr",
                Congestion::Cubic => "cubic",
                Congestion::Reno => "reno",
                Congestion::Newreno => "newreno",
            }
            .into(),
            profile: connection.profile.to_string(),
        }
    }

    fn to_engine(&self) -> Result<ConnectionSettings, GuiError> {
        let mtu = if self.mtu == "auto" {
            Mtu::Auto
        } else {
            Mtu::Value(self.mtu.parse::<u16>().map_err(|_| GuiError {
                code: crate::gui_api::GuiErrorCode::InvalidInput,
                message: "MTU must be auto or an integer".into(),
                retryable: false,
            })?)
        };
        let congestion = match self.congestion.to_ascii_lowercase().as_str() {
            "auto" => Congestion::Auto,
            "bbr" => Congestion::Bbr,
            "cubic" => Congestion::Cubic,
            "reno" => Congestion::Reno,
            "newreno" => Congestion::Newreno,
            _ => {
                return Err(GuiError {
                    code: crate::gui_api::GuiErrorCode::InvalidInput,
                    message: "Choose auto, bbr, cubic, reno or newreno".into(),
                    retryable: false,
                });
            }
        };
        let profile = self
            .profile
            .parse::<hermod_core::options::LinkProfile>()
            .map_err(|_| GuiError {
                code: crate::gui_api::GuiErrorCode::InvalidInput,
                message: "Profile must be auto, lan or wan".into(),
                retryable: false,
            })?;
        Ok(ConnectionSettings {
            transport: self.transport.clone(),
            mtu,
            congestion,
            profile,
        })
    }
}

impl ApiSettings {
    fn from_engine(settings: &crate::app::Settings) -> Self {
        Self {
            version: u32::from(settings.version),
            connection: ApiConnection::from_engine(&settings.connection),
            tuning: settings.tuning.clone().into(),
            receive_transport: settings.receive_transport.clone(),
            ip_family: settings.ip_family.clone(),
            server: settings.server.clone(),
            words: u32::from(settings.words),
            verbose: settings.verbose.clone(),
            quiet: settings.quiet,
            theme: settings.theme.clone(),
            language: settings.language.clone(),
            destination: settings.destination.clone(),
        }
    }

    fn to_engine(&self) -> Result<crate::app::Settings, GuiError> {
        let settings = crate::app::Settings {
            version: u8::try_from(self.version)
                .map_err(|_| GuiError::from_message("Invalid settings version".into()))?,
            connection: self.connection.to_engine()?,
            tuning: self.tuning.clone().into(),
            receive_transport: self.receive_transport.clone(),
            ip_family: self.ip_family.clone(),
            server: self.server.clone(),
            words: u8::try_from(self.words)
                .map_err(|_| GuiError::from_message("Invalid code length".into()))?,
            verbose: self.verbose.clone(),
            quiet: self.quiet,
            theme: self.theme.clone(),
            language: self.language.clone(),
            destination: self.destination.clone(),
        };
        settings.validate().map_err(GuiError::from_message)?;
        Ok(settings)
    }
}

fn api_bootstrap(engine: &BootstrapDto) -> ApiBootstrap {
    ApiBootstrap {
        device_id: engine.device_id.clone(),
        version: engine.version.clone(),
        ipv4: engine.ipv4.clone(),
        ipv6: engine.ipv6.clone(),
        platform: engine.platform.clone(),
        settings_saved: engine.settings_saved,
        settings: ApiSettings::from_engine(&engine.settings),
    }
}

static CONTROLLER: Mutex<Option<Arc<crate::gui_api::GuiController>>> = Mutex::new(None);

fn controller() -> Result<Arc<crate::gui_api::GuiController>, GuiError> {
    CONTROLLER
        .lock()
        .map_err(|_| GuiError::from_message("Controller lock poisoned".into()))?
        .clone()
        .ok_or_else(|| GuiError {
            code: crate::gui_api::GuiErrorCode::Internal,
            message: "GUI is not initialized".into(),
            retryable: false,
        })
}

/// 重复初始化明确拒绝；shutdown 完成后可重新初始化。
pub fn initialize(app_dir: String) -> Result<ApiBootstrap, GuiError> {
    let mut slot = CONTROLLER
        .lock()
        .map_err(|_| GuiError::from_message("Controller lock poisoned".into()))?;
    if slot.is_some() {
        return Err(GuiError::from_message("GUI is already initialized".into()));
    }
    let root = PathBuf::from(app_dir);
    let instance = match crate::gui_api::GuiController::new(root.clone(), Config::default()) {
        Ok(instance) => Arc::new(instance),
        Err(error) => {
            // Diagnostics contain only the stable code, operation/path and OS
            // failure; identity parsing never includes private payload values.
            if let Err(write_error) = std::fs::write(root.join("startup-error.txt"), &error.message)
            {
                tracing::warn!(%write_error, "Could not persist startup diagnostic");
            }
            return Err(error);
        }
    };
    let bootstrap = api_bootstrap(&instance.bootstrap());
    *slot = Some(instance);
    Ok(bootstrap)
}

/// 停止接收新任务、幂等取消并有界等待。未初始化时也成功。
pub fn shutdown() -> Result<(), GuiError> {
    let mut slot = CONTROLLER
        .lock()
        .map_err(|_| GuiError::from_message("Controller lock poisoned".into()))?;
    let result = slot.as_ref().map(|c| c.shutdown()).unwrap_or(Ok(()));
    *slot = None;
    result
}

pub fn devices() -> Result<Vec<crate::gui_api::GuiDeviceDto>, GuiError> {
    controller()?.devices()
}
pub fn rename_device(device_id: String, name: String) -> Result<(), GuiError> {
    controller()?.rename_device(&device_id, &name)
}
pub fn remove_device(device_id: String) -> Result<(), GuiError> {
    controller()?.remove_device(&device_id)
}

pub fn status() -> Option<TransferSnapshot> {
    controller().ok()?.status()
}

pub fn select_source(path: String) -> Result<SelectionDto, GuiError> {
    controller()?.select_file(&PathBuf::from(path))
}

/// 用于平台授权暂存文件；name 是经过 Rust 校验的显示/协议文件名。
pub fn select_named_source(path: String, name: String) -> Result<SelectionDto, GuiError> {
    controller()?.select_file_named(&PathBuf::from(path), name)
}

pub fn start(request: StartRequest) -> Result<(), GuiError> {
    controller()?.start(request)
}

pub fn cancel() {
    if let Ok(controller) = controller() {
        controller.cancel();
    }
}

pub fn inbox() -> Result<Vec<ReceivedDto>, GuiError> {
    controller()?.list_received()
}

pub fn remove(id: String) -> Result<(), GuiError> {
    controller()?.remove_received(&id)
}

pub fn export(id: String, path: String, overwrite: bool) -> Result<(), GuiError> {
    controller()?.export_path(&id, &PathBuf::from(path), overwrite)
}

pub fn probe_server(server: String) -> Result<String, GuiError> {
    controller()?.probe_server(&server)
}

pub fn trust_server(server: String, fingerprint: String) -> Result<(), GuiError> {
    controller()?.trust_server(&server, &fingerprint)
}

pub fn save_settings(settings: ApiSettings) -> Result<(), GuiError> {
    let controller = controller()?;
    controller.save_settings(settings.to_engine()?)
}

pub fn logs() -> Vec<String> {
    controller().map(|c| c.logs()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_conversion_preserves_and_validates_ip_family() {
        use hermod_core::options::IpFamily;

        let mut settings = ApiSettings::from_engine(&crate::app::Settings::default());
        assert_eq!(settings.ip_family, "auto");
        for (value, family) in [
            ("auto", IpFamily::Auto),
            ("ipv4", IpFamily::V4),
            ("ipv6", IpFamily::V6),
        ] {
            settings.ip_family = value.into();
            let engine = settings.to_engine().unwrap();
            assert_eq!(engine.options(false).unwrap().family, family);
            assert_eq!(engine.options(true).unwrap().family, family);
            assert_eq!(ApiSettings::from_engine(&engine).ip_family, value);
        }
        settings.ip_family = "invalid".into();
        assert!(settings.to_engine().is_err());
    }

    #[test]
    fn settings_conversion_rejects_overflow_and_fake_fields() {
        let mut settings = ApiSettings::from_engine(&crate::app::Settings::default());
        settings.words = 65536;
        assert!(settings.to_engine().is_err());
        settings.words = 3;
        settings.connection.profile = "compatible".into();
        assert!(settings.to_engine().is_err());
        settings.connection.profile = "wan".into();
        settings.tuning.streams = Some(0);
        assert!(settings.to_engine().is_err());
        settings.tuning.streams = Some(8);
        assert_eq!(
            settings
                .to_engine()
                .unwrap()
                .options(false)
                .unwrap()
                .tuning
                .streams,
            Some(8)
        );
    }

    #[test]
    fn global_lifecycle_rejects_duplicate_initialize_and_reopens_after_shutdown() {
        let root = std::env::temp_dir().join(format!(
            "hermod-lifecycle-{}",
            crate::app::storage::new_id()
        ));
        shutdown().unwrap();
        let first = initialize(root.to_string_lossy().into_owned()).unwrap();
        assert!(initialize(root.to_string_lossy().into_owned()).is_err());
        shutdown().unwrap();
        shutdown().unwrap();
        assert!(controller().is_err());
        let second = initialize(root.to_string_lossy().into_owned()).unwrap();
        assert_eq!(first.device_id, second.device_id);
        shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
