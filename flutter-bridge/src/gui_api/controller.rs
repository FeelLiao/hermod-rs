//! GUI 唯一业务入口；平台适配器不需要访问 Core 私有目录。
use super::{
    error::{GuiError, GuiResultExt},
    models::*,
};
use crate::app::Engine;
use hermod_core::config::Config;
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone)]
pub struct GuiController {
    engine: Arc<Engine>,
}

impl GuiController {
    pub fn new(root: PathBuf, fallback: Config) -> Result<Self, GuiError> {
        Engine::new(root, fallback)
            .map(|engine| Self {
                engine: Arc::new(engine),
            })
            .gui()
    }

    pub fn bootstrap(&self) -> BootstrapDto {
        self.engine.bootstrap()
    }
    pub fn settings(&self) -> SettingsDto {
        self.engine.settings()
    }
    pub fn save_settings(&self, settings: SettingsDto) -> Result<(), GuiError> {
        self.engine.save_settings(settings).gui()
    }
    pub fn set_destination(&self, destination: Option<String>) -> Result<(), GuiError> {
        self.engine.set_destination(destination).gui()
    }
    pub fn transfer_destination(&self) -> Option<String> {
        self.engine.transfer_destination()
    }
    pub fn ensure_idle(&self) -> Result<(), GuiError> {
        self.engine.ensure_idle().gui()
    }

    pub fn select(
        &self,
        reader: Box<dyn Read + Send>,
        name: String,
        size: Option<u64>,
    ) -> Result<SelectionDto, GuiError> {
        self.engine.select(reader, name, size).gui()
    }
    pub fn select_file(&self, path: &Path) -> Result<SelectionDto, GuiError> {
        self.engine.select_file(path).gui()
    }
    pub fn start(&self, request: StartRequest) -> Result<(), GuiError> {
        self.engine
            .start(request.source_id, request.code, request.text)
            .gui()
    }
    pub fn status(&self) -> Option<TransferSnapshot> {
        self.engine
            .typed_status()
            .map(|(phase, status)| TransferSnapshot::from_parts(phase, status))
    }
    pub fn cancel(&self) {
        self.engine.cancel();
    }

    pub fn list_received(&self) -> Result<Vec<ReceivedDto>, GuiError> {
        self.engine.list_received().gui()
    }
    pub fn received(&self, id: &str) -> Result<ReceivedDto, GuiError> {
        self.engine.received(id).gui()
    }
    pub fn remove_received(&self, id: &str) -> Result<(), GuiError> {
        self.engine.remove(id).gui()
    }
    pub fn export(&self, id: &str, output: &mut dyn Write) -> Result<(), GuiError> {
        self.engine.export(id, output).gui()
    }
    pub fn export_path(&self, id: &str, path: &Path, overwrite: bool) -> Result<(), GuiError> {
        self.engine.export_path(id, path, overwrite).gui()
    }

    pub fn probe_server(&self, server: &str) -> Result<String, GuiError> {
        self.engine.probe_server(server).gui()
    }
    pub fn trust_server(&self, server: &str, fingerprint: &str) -> Result<(), GuiError> {
        self.engine.trust_server(server, fingerprint).gui()
    }
    pub fn logs(&self) -> Vec<String> {
        self.engine.logs()
    }
}
