//! 桌面与 Android 共享的应用层：任务、配置、信任、日志及已验证收件箱。
mod logs;
pub mod storage;
pub mod tuning;
pub mod worker;
use hermod_core::{
    config::{self, Config},
    options::ConnectionSettings,
};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::Duration,
};
use worker::{Action, Job, Source};

pub fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("Runtime unavailable")
    })
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    #[serde(default = "legacy_settings_version")]
    pub version: u8,
    pub connection: ConnectionSettings,
    pub tuning: tuning::TuningSettings,
    pub receive_transport: String,
    pub ip_family: String,
    pub server: String,
    pub words: u8,
    pub verbose: String,
    pub quiet: bool,
    pub theme: String,
    pub language: String,
    pub destination: Option<String>,
}
fn legacy_settings_version() -> u8 {
    1
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 2,
            tuning: Default::default(),
            connection: ConnectionSettings {
                transport: "quic".into(),
                ..Default::default()
            },
            receive_transport: "auto".into(),
            ip_family: "auto".into(),
            server: config::DEFAULT_SERVER_URL.into(),
            words: 3,
            verbose: "none".into(),
            quiet: false,
            theme: "system".into(),
            language: "system".into(),
            destination: None,
        }
    }
}
impl Settings {
    fn family(&self) -> Result<hermod_core::options::IpFamily, String> {
        match self.ip_family.as_str() {
            "auto" => Ok(hermod_core::options::IpFamily::Auto),
            "ipv4" => Ok(hermod_core::options::IpFamily::V4),
            "ipv6" => Ok(hermod_core::options::IpFamily::V6),
            _ => Err("IP family must be auto, ipv4 or ipv6".into()),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        self.options(false)?;
        self.options(true)?;
        if !(1..=32).contains(&self.words) {
            return Err("Code length must be 1..=32 words".into());
        }
        if !["none", "error", "warning", "info", "debug"].contains(&self.verbose.as_str()) {
            return Err("Invalid log level".into());
        }
        if !["system", "light", "dark"].contains(&self.theme.as_str()) {
            return Err("Invalid theme".into());
        }
        if !["system", "en-US", "zh-CN"].contains(&self.language.as_str()) {
            return Err("Invalid language".into());
        }
        config::normalize_server_url(&self.server).map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn options(&self, receiving: bool) -> Result<hermod_core::session::SessionOptions, String> {
        let mut connection = self.connection.clone();
        if receiving {
            connection.transport = self.receive_transport.clone();
        }
        let mut options = connection.session_options(receiving)?;
        options.tuning = self.tuning.validated()?;
        options.family = self.family()?;
        options.validate_for(
            options
                .transport
                .unwrap_or(hermod_core::transport::TransportKind::Quic),
        )?;
        Ok(options)
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Selection {
    pub id: String,
    pub name: String,
    pub size: Option<u64>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Status {
    pub export_error: Option<String>,
    pub phase: String,
    pub code: Option<String>,
    pub bytes: u64,
    pub total: Option<u64>,
    pub elapsed: f64,
    pub done: bool,
    pub cancelled: bool,
    pub error: Option<String>,
    pub digest: Option<String>,
    pub rtt_ms: Option<f64>,
    pub transport: Option<String>,
    pub received_id: Option<String>,
}
#[derive(Serialize)]
pub struct Bootstrap {
    pub device_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_seed: Option<String>,
    pub version: String,
    pub ipv6: Vec<String>,
    pub ipv4: Vec<String>,
    pub platform: String,
    pub settings: Settings,
    pub trusted_servers: std::collections::BTreeMap<String, String>,
    pub settings_saved: bool,
}
/// Latest-value status aggregate. The sender is never cloned: taking it closes
/// every observer, even while a worker still owns a publication callback.
#[derive(Clone)]
pub(crate) struct StateEvent {
    pub sequence: u64,
    pub status: Option<worker::Snapshot>,
}
pub(crate) struct EventHub {
    sender: Mutex<Option<tokio::sync::watch::Sender<StateEvent>>>,
}
impl EventHub {
    fn new() -> Self {
        let (sender, _) = tokio::sync::watch::channel(StateEvent {
            sequence: 0,
            status: None,
        });
        Self {
            sender: Mutex::new(Some(sender)),
        }
    }
    fn publish(&self, status: worker::Snapshot) {
        if let Some(sender) = self.sender.lock().unwrap().as_ref() {
            sender.send_modify(|event| {
                event.sequence = event
                    .sequence
                    .checked_add(1)
                    .expect("GUI sequence exhausted");
                event.status = Some(status);
            });
        }
    }
    pub(crate) fn subscribe(&self) -> Option<tokio::sync::watch::Receiver<StateEvent>> {
        self.sender
            .lock()
            .unwrap()
            .as_ref()
            .map(|sender| sender.subscribe())
    }
    fn close(&self) {
        self.sender.lock().unwrap().take();
    }
}

pub struct Engine {
    identity: std::sync::Arc<hermod_core::identity::Identity>,
    contacts: hermod_core::contacts::Contacts,
    closed: std::sync::atomic::AtomicBool,
    _storage_lock: std::fs::File,
    root: PathBuf,
    pub inbox: PathBuf,
    cache: PathBuf,
    config: Mutex<Config>,
    settings: Mutex<Settings>,
    source: Mutex<Option<(String, Source)>>,
    job: Mutex<Option<Job>>,
    events: std::sync::Arc<EventHub>,
    operation: Mutex<()>,
    transfer_destination: Mutex<Option<String>>,
}
impl Engine {
    pub fn new(root: PathBuf, fallback: Config) -> Result<Self, String> {
        #[cfg(windows)]
        crate::windows::initialize_storage()?;
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }

        let storage_lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join("storage.lock"))
            .map_err(|e| e.to_string())?;
        storage_lock
            .try_lock()
            .map_err(|_| "Another Hermod instance is using this inbox")?;
        let inbox = root.join("received");
        let cache = root.join("outgoing");
        std::fs::create_dir_all(&inbox).map_err(|e| e.to_string())?;
        storage::clean_incomplete(&inbox)?;
        if cache.exists() {
            std::fs::remove_dir_all(&cache).map_err(|e| e.to_string())?;
        }
        std::fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
        let mut settings = if root.join("settings.json").exists() {
            serde_json::from_slice::<Settings>(
                &std::fs::read(root.join("settings.json")).map_err(|e| e.to_string())?,
            )
            .map_err(|e| format!("Could not read settings: {e}"))?
        } else {
            Settings {
                server: fallback.server_url.clone(),
                ..Default::default()
            }
        };
        let mut cfg = if root.join("config.yaml").exists() {
            config::load_selected(&root.join("config.yaml"), Some(&settings.server))
                .map_err(|e| e.to_string())?
        } else {
            fallback
        };
        cfg.retain_active_trust().map_err(|e| e.to_string())?;
        settings.server = cfg.server_url.clone();
        settings.validate()?;
        config::save_to(&cfg, &root.join("config.yaml")).map_err(|e| e.to_string())?;
        let identity =
            hermod_core::identity::Identity::load_or_create(&root).map_err(|e| e.to_string())?;
        let contacts = hermod_core::contacts::Contacts::open(&root).map_err(|e| e.to_string())?;
        logs::initialize();
        logs::set_level(&settings.verbose);
        Ok(Self {
            identity: std::sync::Arc::new(identity),
            contacts,
            closed: std::sync::atomic::AtomicBool::new(false),
            _storage_lock: storage_lock,
            root,
            inbox,
            cache,
            config: Mutex::new(cfg),
            settings: Mutex::new(settings),
            source: Mutex::new(None),
            job: Mutex::new(None),
            events: std::sync::Arc::new(EventHub::new()),
            operation: Mutex::new(()),
            transfer_destination: Mutex::new(None),
        })
    }
    pub fn settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }
    pub fn bootstrap(&self) -> Bootstrap {
        Bootstrap {
            device_id: self.identity.device_id().to_string(),
            color_seed: None,
            version: env!("CARGO_PKG_VERSION").into(),
            ipv4: hermod_core::transport::network::local_ipv4()
                .iter()
                .map(ToString::to_string)
                .collect(),
            ipv6: hermod_core::transport::quic::addr::enumerate_local_ipv6()
                .iter()
                .map(ToString::to_string)
                .collect(),
            platform: std::env::consts::OS.into(),
            settings: self.settings(),
            trusted_servers: self.config.lock().unwrap().trusted_servers.clone(),
            settings_saved: self.root.join("settings.json").exists(),
        }
    }
    pub fn save_settings(&self, mut settings: Settings) -> Result<(), String> {
        settings.validate()?;
        settings.version = 2;
        let mut current = self.settings.lock().unwrap();
        if config::normalize_server_url(&settings.server).map_err(|e| e.to_string())?
            != self.config.lock().unwrap().server_url
        {
            return Err("Verify and trust the new server before saving settings".into());
        }
        if settings.destination != current.destination {
            return Err("Choose a destination using the system picker".into());
        }
        self.persist_settings(&settings)?;
        logs::set_level(&settings.verbose);
        *current = settings;
        Ok(())
    }
    fn persist_settings(&self, settings: &Settings) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?;
        // 锁内写入临时文件；rename 在支持替换的平台上保持原子性。
        let temporary = self.root.join("settings.tmp");
        std::fs::write(&temporary, bytes).map_err(|e| e.to_string())?;
        std::fs::rename(temporary, self.root.join("settings.json")).map_err(|e| e.to_string())
    }
    pub fn set_destination(&self, value: Option<String>) -> Result<(), String> {
        let mut settings = self.settings.lock().unwrap();
        let mut updated = settings.clone();
        updated.destination = value;
        updated.validate()?;
        self.persist_settings(&updated)?;
        *settings = updated;
        Ok(())
    }
    pub fn ensure_idle(&self) -> Result<(), String> {
        if self.closed.load(std::sync::atomic::Ordering::Acquire) {
            return Err("GUI engine is shut down".into());
        }
        if self.status().is_some_and(|s| !s.done) {
            return Err("A transfer is already running".into());
        }
        Ok(())
    }
    pub fn select(
        &self,
        reader: Box<dyn Read + Send>,
        name: String,
        size: Option<u64>,
    ) -> Result<Selection, String> {
        self.select_inner(reader, name, size, None)
    }

    fn select_inner(
        &self,
        reader: Box<dyn Read + Send>,
        name: String,
        size: Option<u64>,
        path: Option<PathBuf>,
    ) -> Result<Selection, String> {
        let _operation = self
            .operation
            .try_lock()
            .map_err(|_| "Another file operation is in progress")?;
        self.ensure_idle()?;
        hermod_core::transfer::safe_destination_path(&self.cache, &name)
            .map_err(|e| e.to_string())?;
        let id = storage::new_id();
        let selection = Selection {
            id: id.clone(),
            name: name.clone(),
            size,
        };
        *self.source.lock().unwrap() = Some((
            id,
            Source {
                reader,
                name,
                size,
                cache: self.cache.join(storage::new_id()),
                kind: hermod_core::transfer::Kind::File,
                path,
            },
        ));
        Ok(selection)
    }
    pub fn select_file(&self, path: &Path) -> Result<Selection, String> {
        let name = path
            .file_name()
            .ok_or("Invalid filename")?
            .to_string_lossy()
            .into_owned();
        self.select_file_named(path, name)
    }
    /// 授权来源暂存路径与协议文件名分离，保留 seek 能力。
    pub fn select_file_named(&self, path: &Path, name: String) -> Result<Selection, String> {
        if name.is_empty()
            || name == "."
            || name == ".."
            || name.contains(['/', '\\', '\0'])
            || name.chars().any(char::is_control)
        {
            return Err("Invalid filename".into());
        }
        hermod_core::transfer::safe_destination_path(&self.cache, &name)
            .map_err(|e| e.to_string())?;
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        if !metadata.is_file() {
            return Err("Select a regular file".into());
        }
        self.select_inner(
            Box::new(file),
            name,
            Some(metadata.len()),
            Some(path.to_path_buf()),
        )
    }
    pub fn start(
        &self,
        source_id: Option<String>,
        code: Option<String>,
        text: Option<String>,
    ) -> Result<(), String> {
        self.start_inner(source_id, code, text, None, false, false)
    }
    pub fn start_request(&self, request: crate::gui_api::StartRequest) -> Result<(), String> {
        if request.text.is_some() {
            return Err("GUI text transfers are not supported".into());
        }
        self.start_inner(
            request.source_id,
            request.code,
            None,
            request.from_device,
            request.pair,
            true,
        )
    }
    fn start_inner(
        &self,
        source_id: Option<String>,
        code: Option<String>,
        text: Option<String>,
        from_device: Option<String>,
        pair: bool,
        device_mode: bool,
    ) -> Result<(), String> {
        let _operation = self
            .operation
            .try_lock()
            .map_err(|_| "Another file operation is in progress")?;
        self.ensure_idle()?;
        if usize::from(source_id.is_some())
            + usize::from(code.is_some())
            + usize::from(text.is_some())
            + usize::from(from_device.is_some())
            != 1
        {
            return Err("Choose a file, text, or a receive code".into());
        }
        // 与替换信任保持相同锁顺序，使每个任务使用同一次服务器/指纹快照。
        let (settings, mut config) = {
            let settings = self.settings.lock().unwrap();
            (settings.clone(), self.config.lock().unwrap().clone())
        };
        settings.validate()?;
        let receiving = code.is_some() || from_device.is_some();
        let options = settings.options(receiving)?;
        config.server_url =
            config::normalize_server_url(&settings.server).map_err(|e| e.to_string())?;
        hermod_core::session::expected_spki(&config).map_err(|e| e.to_string())?;
        let device = device_mode.then(|| worker::DeviceContext {
            identity: self.identity.clone(),
            contacts: self.contacts.clone(),
            pair,
        });
        let action = if let Some(target) = from_device {
            let id = target
                .parse::<hermod_core::identity::DeviceId>()
                .map_err(|e| e.to_string())?;
            let peer = self.contacts.lookup(id).map_err(|e| e.to_string())?;
            let directory = self.inbox.join(storage::new_id()).join("payload");
            std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
            Action::ReceiveDevice { peer, directory }
        } else if let Some(code) = code {
            let code = code.trim().to_string();
            hermod_core::crypto::wordlist::parse_transfer_code(&code).map_err(|e| e.to_string())?;
            let directory = self.inbox.join(storage::new_id()).join("payload");
            std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
            Action::Receive { code, directory }
        } else if let Some(text) = text {
            if text.is_empty() || text.len() > 1024 * 1024 {
                return Err("Text must contain 1 byte to 1 MiB".into());
            }
            Action::Send(Source {
                size: Some(text.len() as u64),
                reader: Box::new(std::io::Cursor::new(text.into_bytes())),
                name: "message.txt".into(),
                kind: hermod_core::transfer::Kind::Text,
                cache: self.cache.join(storage::new_id()),
                path: None,
            })
        } else {
            let mut selected = self.source.lock().unwrap();
            if selected
                .as_ref()
                .is_none_or(|(id, _)| Some(id) != source_id.as_ref())
            {
                return Err("Choose the file again".into());
            }
            Action::Send(selected.take().unwrap().1)
        };
        *self.transfer_destination.lock().unwrap() = settings.destination.clone();
        let _runtime = runtime().enter();
        let events = self.events.clone();
        *self.job.lock().unwrap() = Some(Job::start_observed(
            config,
            options,
            action,
            settings.words,
            Box::new(|| {}),
            device,
            Box::new(move |snapshot| events.publish(snapshot)),
        ));
        Ok(())
    }
    pub fn devices(&self) -> Result<Vec<crate::gui_api::GuiDeviceDto>, String> {
        self.contacts
            .list()
            .map(|items| {
                items
                    .into_iter()
                    .map(|c| crate::gui_api::GuiDeviceDto {
                        device_id: c.device_id.to_string(),
                        name: c.name,
                        created_at: c.created_at,
                        updated_at: c.updated_at,
                        active: c.state == hermod_core::contacts::PairingState::Active,
                    })
                    .collect()
            })
            .map_err(|e| e.to_string())
    }
    pub fn rename_device(&self, id: &str, name: &str) -> Result<(), String> {
        self.contacts
            .rename(
                id.parse()
                    .map_err(|e: hermod_core::identity::Error| e.to_string())?,
                name,
            )
            .map_err(|e| e.to_string())
    }
    pub fn remove_device(&self, id: &str) -> Result<(), String> {
        self.contacts
            .remove(
                id.parse()
                    .map_err(|e: hermod_core::identity::Error| e.to_string())?,
            )
            .map_err(|e| e.to_string())
    }
    pub fn shutdown(&self) -> Result<(), String> {
        let _operation = self
            .operation
            .lock()
            .map_err(|_| "Operation lock poisoned")?;
        self.closed
            .store(true, std::sync::atomic::Ordering::Release);
        let job = self.job.lock().map_err(|_| "Job lock poisoned")?.take();
        let result = if let Some(mut job) = job {
            runtime().block_on(job.shutdown(Duration::from_secs(3)))
        } else {
            Ok(())
        };
        self.events.close();
        result
    }
    pub(crate) fn export_cache(&self) -> PathBuf {
        self.root.join("export-cache")
    }
    pub fn transfer_destination(&self) -> Option<String> {
        self.transfer_destination.lock().unwrap().clone()
    }
    pub(crate) fn subscribe_status(&self) -> Option<tokio::sync::watch::Receiver<StateEvent>> {
        self.events.subscribe()
    }
    pub fn status(&self) -> Option<Status> {
        self.typed_status().map(|(_, status)| status)
    }
    /// 返回类型化阶段与兼容旧 GUI 的状态结构。
    pub fn typed_status(&self) -> Option<(worker::TransferPhase, Status)> {
        self.job
            .lock()
            .unwrap()
            .as_ref()
            .map(|job| Self::snapshot_status(job.snapshot()))
    }
    pub(crate) fn snapshot_status(s: worker::Snapshot) -> (worker::TransferPhase, Status) {
        let phase = s.phase;
        let status = Status {
            export_error: None,
            elapsed: s
                .completed
                .unwrap_or_else(std::time::Instant::now)
                .duration_since(s.started)
                .as_secs_f64(),
            done: s.completed.is_some(),
            phase: s.phase.message().into(),
            code: s.code,
            bytes: s.bytes,
            total: s.total,
            cancelled: s.cancelled,
            error: s.error,
            digest: s.digest,
            rtt_ms: s.network.map(|n| n.snapshot().rtt.as_secs_f64() * 1000.0),
            transport: s.transport,
            received_id: s.path.and_then(|p| {
                p.parent()?
                    .parent()?
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
            }),
        };
        (phase, status)
    }
    pub fn cancel(&self) {
        if let Some(job) = self.job.lock().unwrap().as_ref() {
            job.cancel();
        }
    }
    pub fn list_received(&self) -> Result<Vec<storage::Received>, String> {
        storage::list(&self.inbox)
    }
    /// 查询收件箱元数据，不向 GUI 暴露私有文件路径。
    pub fn received(&self, id: &str) -> Result<storage::Received, String> {
        storage::resolve(&self.inbox, id).map(|(item, _)| item)
    }
    pub fn remove(&self, id: &str) -> Result<(), String> {
        let _guard = self
            .operation
            .try_lock()
            .map_err(|_| "Another file operation is in progress")?;
        self.ensure_idle()?;
        storage::remove(&self.inbox, id)
    }
    pub fn export(&self, id: &str, output: &mut dyn Write) -> Result<(), String> {
        let _guard = self
            .operation
            .try_lock()
            .map_err(|_| "Another file operation is in progress")?;
        self.ensure_idle()?;
        self.copy_verified(id, output)
    }
    fn copy_verified(&self, id: &str, output: &mut dyn Write) -> Result<(), String> {
        let (item, path) = storage::resolve(&self.inbox, id)?;
        let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let copied = std::io::copy(&mut file, output)
            .map_err(|e| format!("Export failed; inbox copy retained: {e}"))?;
        if copied != item.size {
            return Err("Export size mismatch; inbox copy retained".into());
        }
        output.flush().map_err(|e| e.to_string())
    }
    pub fn export_path(&self, id: &str, path: &Path, overwrite: bool) -> Result<(), String> {
        let _guard = self
            .operation
            .try_lock()
            .map_err(|_| "Another file operation is in progress")?;
        self.ensure_idle()?;
        let (_, source) = storage::resolve(&self.inbox, id)?;
        if path
            .canonicalize()
            .is_ok_and(|p| p == source || p.starts_with(&self.inbox))
        {
            return Err("Choose a path outside the private inbox".into());
        }
        self.ensure_idle()?;
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create(overwrite)
            .truncate(overwrite)
            .create_new(!overwrite)
            .open(path)
            .map_err(|e| e.to_string())?;
        self.copy_verified(id, &mut output)?;
        output.sync_all().map_err(|e| e.to_string())
    }
    pub fn probe_server(&self, server: &str) -> Result<String, String> {
        let family = self.settings.lock().unwrap().family()?;
        let url = config::normalize_server_url(server).map_err(|e| e.to_string())?;
        runtime().block_on(async {
            let fp = tokio::time::timeout(
                Duration::from_secs(15),
                hermod_core::signal::tls::fetch_server_spki_with_family(&url, family),
            )
            .await
            .map_err(|_| "Server certificate request timed out")?
            .map_err(|e| e.to_string())?;
            Ok(hermod_core::transfer::hex(&fp))
        })
    }
    pub fn trust_server(&self, server: &str, expected: &str) -> Result<(), String> {
        if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Expected a SHA-256 SPKI fingerprint (64 hexadecimal characters)".into());
        }
        let fingerprint = self.probe_server(server)?;
        if !fingerprint.eq_ignore_ascii_case(expected) {
            return Err("Server fingerprint changed or does not match; trust was not saved".into());
        }
        let url = config::normalize_server_url(server).map_err(|e| e.to_string())?;
        let mut settings = self.settings.lock().unwrap();
        let mut cfg = self.config.lock().unwrap();
        let mut updated = cfg.clone();
        updated
            .replace_trust(&url, &fingerprint)
            .map_err(|e| e.to_string())?;
        config::save_to(&updated, &self.root.join("config.yaml")).map_err(|e| e.to_string())?;
        *cfg = updated;
        settings.server = url;
        Ok(())
    }
    pub fn logs(&self) -> Vec<String> {
        logs::read()
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.events.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ip_family_defaults_maps_persists_and_rejects_invalid_values() {
        use hermod_core::options::IpFamily;

        let legacy: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(legacy.ip_family, "auto");
        let root = std::env::temp_dir().join(format!("hermod-family-{}", storage::new_id()));
        let engine = Engine::new(root.clone(), Config::default()).unwrap();
        for (value, family) in [
            ("auto", IpFamily::Auto),
            ("ipv4", IpFamily::V4),
            ("ipv6", IpFamily::V6),
        ] {
            let mut settings = engine.settings();
            settings.ip_family = value.into();
            for receiving in [false, true] {
                assert_eq!(settings.options(receiving).unwrap().family, family);
            }
            engine.save_settings(settings).unwrap();
        }
        let saved = std::fs::read(root.join("settings.json")).unwrap();
        for value in ["", "IPv4", "v6", "dual", "ipv4 "] {
            let mut settings = engine.settings();
            settings.ip_family = value.into();
            assert!(settings.options(false).is_err());
            assert!(settings.options(true).is_err());
            assert!(engine.save_settings(settings).is_err());
            assert_eq!(std::fs::read(root.join("settings.json")).unwrap(), saved);
            assert_eq!(engine.settings().ip_family, "ipv6");
        }
        drop(engine);
        let engine = Engine::new(root.clone(), Config::default()).unwrap();
        assert_eq!(engine.settings().ip_family, "ipv6");
        assert_eq!(
            engine.settings().options(false).unwrap().family,
            IpFamily::V6
        );
        // 字面量与设置族冲突应在联网前失败；trust 必须使用同一 probe 路径。
        let error = engine.probe_server("wss://127.0.0.1:1").unwrap_err();
        assert!(error.contains("family"), "{error}");
        let error = engine
            .trust_server("wss://127.0.0.1:1", &"ab".repeat(32))
            .unwrap_err();
        assert!(error.contains("family"), "{error}");
        let mut invalid = engine.settings();
        invalid.ip_family = "invalid".into();
        drop(engine);
        std::fs::write(
            root.join("settings.json"),
            serde_json::to_vec(&invalid).unwrap(),
        )
        .unwrap();
        let error = match Engine::new(root.clone(), Config::default()) {
            Err(error) => error,
            Ok(_) => panic!("invalid persisted IP family must be rejected"),
        };
        assert!(error.contains("IP family"), "{error}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn engine_idle_snapshot_and_shutdown_close_subscriptions() {
        let root = std::env::temp_dir().join(format!("hermod-events-{}", storage::new_id()));
        let engine = Engine::new(root.clone(), Config::default()).unwrap();
        let mut receiver = engine.subscribe_status().unwrap();
        assert_eq!(receiver.borrow_and_update().sequence, 0);
        assert!(receiver.borrow().status.is_none());
        engine.shutdown().unwrap();
        runtime().block_on(async {
            assert!(receiver.changed().await.is_err());
        });
        assert!(engine.subscribe_status().is_none());
        drop(engine);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn gui_requests_default_to_pairing_reject_text_and_preserve_cli_entry() {
        let request = crate::gui_api::StartRequest::default();
        assert!(request.pair);
        assert!(
            serde_json::from_str::<crate::gui_api::StartRequest>("{}")
                .unwrap()
                .pair
        );
        let root = std::env::temp_dir().join(format!("hermod-contract-{}", storage::new_id()));
        let engine = Engine::new(root.clone(), Config::default()).unwrap();
        let id = engine.bootstrap().device_id;
        assert_eq!(id.len(), 32);
        assert!(engine.devices().unwrap().is_empty());
        let peer_root = root.join("peer-fixture");
        let peer = hermod_core::identity::Identity::load_or_create(&peer_root).unwrap();
        engine
            .contacts
            .stage_pairing(
                peer.device_id(),
                peer.public_key(),
                "",
                "test-pair",
                [7; 32],
            )
            .unwrap();
        assert!(!engine.devices().unwrap()[0].active);
        assert!(engine.contacts.lookup(peer.device_id()).is_err());
        engine
            .contacts
            .confirm_pairing(peer.device_id(), "test-pair")
            .unwrap();
        engine
            .rename_device(&peer.device_id().to_string(), "desktop")
            .unwrap();
        assert_eq!(engine.devices().unwrap()[0].name, "desktop");
        assert!(engine.devices().unwrap()[0].active);
        assert!(
            engine
                .rename_device(&peer.device_id().to_string(), "bad\nname")
                .is_err()
        );
        engine.remove_device(&peer.device_id().to_string()).unwrap();
        assert!(engine.devices().unwrap().is_empty());
        let error = engine
            .start_request(crate::gui_api::StartRequest {
                text: Some("not GUI".into()),
                ..Default::default()
            })
            .unwrap_err();
        assert!(error.contains("text"));
        assert!(
            engine
                .start_request(crate::gui_api::StartRequest {
                    from_device: Some(id.clone()),
                    ..Default::default()
                })
                .is_err()
        );
        engine.shutdown().unwrap();
        engine.shutdown().unwrap();
        assert!(
            engine
                .start(None, None, Some("CLI interop".into()))
                .is_err()
        );
        drop(engine);
        let reopened = Engine::new(root.clone(), Config::default()).unwrap();
        assert_eq!(reopened.bootstrap().device_id, id);
        drop(reopened);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn staged_source_keeps_authorized_name_and_seekable_path() {
        let root = std::env::temp_dir().join(format!("hermod-named-source-{}", storage::new_id()));
        let engine = Engine::new(root.clone(), Config::default()).unwrap();
        let path = root.join("opaque-random-staged");
        std::fs::write(&path, b"file").unwrap();
        let selected = engine
            .select_file_named(&path, "原始文件.zip".into())
            .unwrap();
        assert_eq!(selected.name, "原始文件.zip");
        assert_eq!(selected.size, Some(4));
        let source = engine.source.lock().unwrap();
        assert_eq!(
            source.as_ref().unwrap().1.path.as_deref(),
            Some(path.as_path())
        );
        drop(source);
        assert!(engine.select_file_named(&path, "../escape".into()).is_err());
        assert!(
            engine
                .select_file_named(&root, "directory.zip".into())
                .is_err()
        );
        drop(engine);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tuning_is_validated_before_save_and_survives_reopen() {
        let root = std::env::temp_dir().join(format!("hermod-tuning-{}", storage::new_id()));
        let engine = Engine::new(root.clone(), Config::default()).unwrap();
        let mut settings = engine.settings();
        settings.tuning.streams = Some(4);
        settings.tuning.chunk_bytes = 2 * 1024 * 1024;
        settings.tuning.keepalive_secs = 0;
        engine.save_settings(settings.clone()).unwrap();
        let saved = std::fs::read(root.join("settings.json")).unwrap();
        settings.tuning.streams = Some(9);
        assert!(engine.save_settings(settings.clone()).is_err());
        assert_eq!(std::fs::read(root.join("settings.json")).unwrap(), saved);
        settings.tuning.streams = Some(4);
        settings.tuning.keepalive_secs = 30;
        assert!(engine.save_settings(settings).is_err());
        drop(engine);
        let engine = Engine::new(root.clone(), Config::default()).unwrap();
        assert_eq!(
            engine.settings().options(false).unwrap().tuning.streams,
            Some(4)
        );
        assert_eq!(
            engine.settings().options(false).unwrap().tuning.chunk,
            2 * 1024 * 1024
        );
        drop(engine);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_settings_keep_preferences_and_migrate_the_selected_trust() {
        let root = std::env::temp_dir().join(format!("hermod-migration-{}", storage::new_id()));
        std::fs::create_dir_all(&root).unwrap();
        let server = "wss://selected.example:4376";
        let mut config = Config {
            trust_version: 0,
            ..Default::default()
        };
        config
            .trusted_servers
            .insert(server.into(), "ab".repeat(32));
        // JSON 是合法 YAML；保留旧多信任布局以覆盖真实加载迁移。
        std::fs::write(
            root.join("config.yaml"),
            serde_json::to_vec(&config).unwrap(),
        )
        .unwrap();
        let settings = Settings {
            server: server.into(),
            connection: ConnectionSettings {
                transport: "quic".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        let mut legacy = serde_json::to_value(settings).unwrap();
        legacy.as_object_mut().unwrap().remove("version");
        std::fs::write(
            root.join("settings.json"),
            serde_json::to_vec(&legacy).unwrap(),
        )
        .unwrap();
        let engine = Engine::new(root.clone(), Config::default()).unwrap();
        let settings = engine.settings();
        assert_eq!(settings.version, 1);
        assert_eq!(settings.connection.transport, "quic");
        assert_eq!(settings.server, server);
        assert_eq!(engine.bootstrap().trusted_servers.len(), 1);
        drop(engine);

        // "不迁移"策略：旧版 transport:"wireguard" 在加载时必须显式报错，而不是悄悄换回 QUIC。
        let hostile_root =
            std::env::temp_dir().join(format!("hermod-legacy-wg-{}", storage::new_id()));
        std::fs::create_dir_all(&hostile_root).unwrap();
        let mut legacy_wg = serde_json::to_value(Settings {
            server: server.into(),
            connection: ConnectionSettings {
                transport: "wireguard".into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap();
        legacy_wg.as_object_mut().unwrap().remove("version");
        std::fs::write(
            hostile_root.join("settings.json"),
            serde_json::to_vec(&legacy_wg).unwrap(),
        )
        .unwrap();
        let error = match Engine::new(hostile_root.clone(), Config::default()) {
            Err(error) => error,
            Ok(_) => panic!("legacy wireguard settings must be rejected"),
        };
        assert!(
            error.contains("quic") || error.to_lowercase().contains("transport"),
            "{error}"
        );
        drop(error);
        std::fs::remove_dir_all(hostile_root).unwrap();

        let mut config = config::load_from(&root.join("config.yaml")).unwrap();
        assert_eq!(config.trusted_servers.len(), 1);
        config
            .replace_trust("replacement.example", &"cd".repeat(32))
            .unwrap();
        config::save_to(&config, &root.join("config.yaml")).unwrap();
        let engine = Engine::new(root.clone(), Config::default()).unwrap();
        assert_eq!(engine.settings().server, "wss://replacement.example:4376");
        assert!(engine.save_settings(settings).is_err());
        drop(engine);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn settings_persist_and_reject_invalid_or_unapproved_values() {
        let root = std::env::temp_dir().join(format!("hermod-settings-{}", storage::new_id()));
        let engine = Engine::new(root.clone(), Config::default()).unwrap();
        assert!(Engine::new(root.clone(), Config::default()).is_err());
        let mut settings = engine.settings();
        assert_eq!(settings.connection.transport, "quic");
        assert_eq!(settings.language, "system");
        // 旧设置没有语言字段时仍可加载，并默认跟随系统。
        let mut legacy = serde_json::to_value(&settings).unwrap();
        legacy.as_object_mut().unwrap().remove("language");
        assert_eq!(
            serde_json::from_value::<Settings>(legacy).unwrap().language,
            "system"
        );
        settings.language = "zh-CN".into();
        settings.words = 5;
        settings.quiet = true;
        engine.save_settings(settings.clone()).unwrap();
        let mut invalid_language = settings.clone();
        invalid_language.language = "unsupported".into();
        assert!(engine.save_settings(invalid_language).is_err());
        settings.connection.mtu = "1100".parse().unwrap();
        settings.connection.transport = "quic".into();
        assert!(engine.save_settings(settings).is_err());
        let mut settings = engine.settings();
        settings.destination = Some("/unapproved".into());
        assert!(engine.save_settings(settings).is_err());
        drop(engine);
        let engine = Engine::new(root.clone(), Config::default()).unwrap();
        assert_eq!(engine.settings().words, 5);
        assert!(engine.settings().quiet);
        assert_eq!(engine.settings().language, "zh-CN");
        drop(engine);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn failed_export_preserves_receipt_and_existing_destination() {
        let root = std::env::temp_dir().join(format!("hermod-export-{}", storage::new_id()));
        let engine = Engine::new(root.clone(), Config::default()).unwrap();
        let id = storage::new_id();
        let directory = engine.inbox.join(&id).join("payload");
        std::fs::create_dir_all(&directory).unwrap();
        let source = directory.join("example.txt");
        std::fs::write(&source, b"verified").unwrap();
        runtime()
            .block_on(storage::record(
                &directory,
                &hermod_core::orchestrate::Receipt {
                    path: source.clone(),
                    size: 8,
                    sha256: hermod_core::transfer::hash_bytes(b"verified"),
                    kind: hermod_core::transfer::Kind::File,
                },
            ))
            .unwrap();
        let destination = root.join("existing.txt");
        std::fs::write(&destination, b"keep me").unwrap();
        assert!(engine.export_path(&id, &destination, false).is_err());
        assert_eq!(std::fs::read(destination).unwrap(), b"keep me");
        struct Failed;
        impl Write for Failed {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("Permission revoked"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        assert!(engine.export(&id, &mut Failed).is_err());
        assert_eq!(engine.list_received().unwrap().len(), 1);
        assert_eq!(std::fs::read(source).unwrap(), b"verified");
        drop(engine);
        std::fs::remove_dir_all(root).unwrap();
    }
}
