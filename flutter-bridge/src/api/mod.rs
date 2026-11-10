//! flutter_rust_bridge 暴露的 GUI 能力面。
//!
//! 只转发 [`crate::gui_api::GuiController`]；传输、信任、私有收件箱路径
//! 仍由 Engine 管理，此处仅限类型化 DTO 与不透明 id。
use hermod_core::config::Config;
use hermod_core::options::{Congestion, ConnectionSettings, Mtu};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use crate::gui_api::{
    BootstrapDto, GuiError, ReceivedDto, SelectionDto, StartRequest, TransferSnapshot,
};

#[derive(Debug, Clone)]
pub struct ApiConnection {
    pub transport: String,
    pub mtu: String,
    pub congestion: String,
    /// 链路档位：auto / lan / compatible。
    pub profile: String,
    pub limit_mbps: String,
}

#[derive(Debug, Clone)]
pub struct ApiSettings {
    pub version: u32,
    pub connection: ApiConnection,
    pub receive_transport: String,
    pub server: String,
    pub words: u32,
    pub verbose: String,
    pub quiet: bool,
    pub theme: String,
    pub language: String,
    pub destination: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ApiBootstrap {
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
            limit_mbps: "0".into(),
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
                message: "Profile must be auto, lan or compatible".into(),
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
            receive_transport: settings.receive_transport.clone(),
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
        Ok(crate::app::Settings {
            version: self.version.min(u8::MAX as u32) as u8,
            connection: self.connection.to_engine()?,
            receive_transport: self.receive_transport.clone(),
            server: self.server.clone(),
            words: self.words.min(u8::MAX as u32) as u8,
            verbose: self.verbose.clone(),
            quiet: self.quiet,
            theme: self.theme.clone(),
            language: self.language.clone(),
            destination: self.destination.clone(),
        })
    }
}

fn api_bootstrap(engine: &BootstrapDto) -> ApiBootstrap {
    ApiBootstrap {
        version: engine.version.clone(),
        ipv4: engine.ipv4.clone(),
        ipv6: engine.ipv6.clone(),
        platform: engine.platform.clone(),
        settings_saved: engine.settings_saved,
        settings: ApiSettings::from_engine(&engine.settings),
    }
}

static CONTROLLER: OnceLock<Arc<crate::gui_api::GuiController>> = OnceLock::new();

fn controller() -> Result<Arc<crate::gui_api::GuiController>, GuiError> {
    CONTROLLER.get().cloned().ok_or_else(|| GuiError {
        code: crate::gui_api::GuiErrorCode::Internal,
        message: "GUI is not initialized".into(),
        retryable: false,
    })
}

/// GUI 装载后首先调用；`app_dir` 是 Flutter 传入的应用支持目录。
pub fn initialize(app_dir: String) -> Result<ApiBootstrap, GuiError> {
    let controller = crate::gui_api::GuiController::new(PathBuf::from(app_dir), Config::default())?;
    let arc = Arc::new(controller);
    // GUI 只有单个 Dart isolate 的调用入口；重复初始化时保留首个引擎。
    let _ = CONTROLLER.set(arc.clone());
    Ok(api_bootstrap(&arc.bootstrap()))
}

pub fn status() -> Option<TransferSnapshot> {
    controller().ok()?.status()
}

pub fn select_source(path: String) -> Result<SelectionDto, GuiError> {
    controller()?.select_file(&PathBuf::from(path))
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
