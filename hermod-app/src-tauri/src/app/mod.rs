//! 桌面与 Android 共享的应用层：任务、配置、信任、日志及已验证收件箱。
mod logs;
pub mod storage;
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
    pub connection: ConnectionSettings,
    pub receive_transport: String,
    pub server: String,
    pub words: u8,
    pub verbose: String,
    pub quiet: bool,
    pub theme: String,
    pub language: String,
    pub destination: Option<String>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            connection: ConnectionSettings {
                limit_mbps: "8".into(),
                ..Default::default()
            },
            receive_transport: "auto".into(),
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
        connection.session_options(receiving)
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_seed: Option<String>,
    pub version: String,
    pub ipv6: Vec<String>,
    pub platform: String,
    pub settings: Settings,
    pub trusted_servers: std::collections::BTreeMap<String, String>,
    pub settings_saved: bool,
}
pub struct Engine {
    _storage_lock: std::fs::File,
    root: PathBuf,
    pub inbox: PathBuf,
    cache: PathBuf,
    config: Mutex<Config>,
    settings: Mutex<Settings>,
    source: Mutex<Option<(String, Source)>>,
    job: Mutex<Option<Job>>,
    operation: Mutex<()>,
    transfer_destination: Mutex<Option<String>>,
}
impl Engine {
    pub fn new(root: PathBuf, fallback: Config) -> Result<Self, String> {
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
        let cfg = if root.join("config.yaml").exists() {
            config::load_from(&root.join("config.yaml")).map_err(|e| e.to_string())?
        } else {
            fallback
        };
        let settings = if root.join("settings.json").exists() {
            serde_json::from_slice::<Settings>(
                &std::fs::read(root.join("settings.json")).map_err(|e| e.to_string())?,
            )
            .map_err(|e| format!("Could not read settings: {e}"))?
        } else {
            Settings {
                server: cfg.server_url.clone(),
                ..Default::default()
            }
        };
        settings.validate()?;
        logs::initialize();
        logs::set_level(&settings.verbose);
        Ok(Self {
            _storage_lock: storage_lock,
            root,
            inbox,
            cache,
            config: Mutex::new(cfg),
            settings: Mutex::new(settings),
            source: Mutex::new(None),
            job: Mutex::new(None),
            operation: Mutex::new(()),
            transfer_destination: Mutex::new(None),
        })
    }
    pub fn settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }
    pub fn bootstrap(&self) -> Bootstrap {
        Bootstrap {
            color_seed: None,
            version: env!("CARGO_PKG_VERSION").into(),
            ipv6: hermod_core::transport::wireguard::addr::enumerate_local_ipv6()
                .iter()
                .map(ToString::to_string)
                .collect(),
            platform: std::env::consts::OS.into(),
            settings: self.settings(),
            trusted_servers: self.config.lock().unwrap().trusted_servers.clone(),
            settings_saved: self.root.join("settings.json").exists(),
        }
    }
    pub fn save_settings(&self, settings: Settings) -> Result<(), String> {
        settings.validate()?;
        let mut current = self.settings.lock().unwrap();
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
        self.persist_settings(&updated)?;
        *settings = updated;
        Ok(())
    }
    pub fn ensure_idle(&self) -> Result<(), String> {
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
            },
        ));
        Ok(selection)
    }
    pub fn select_file(&self, path: &Path) -> Result<Selection, String> {
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        if !metadata.is_file() {
            return Err("Select a regular file".into());
        }
        self.select(
            Box::new(file),
            path.file_name()
                .ok_or("Invalid filename")?
                .to_string_lossy()
                .into_owned(),
            Some(metadata.len()),
        )
    }
    pub fn start(
        &self,
        source_id: Option<String>,
        code: Option<String>,
        text: Option<String>,
    ) -> Result<(), String> {
        let _operation = self
            .operation
            .try_lock()
            .map_err(|_| "Another file operation is in progress")?;
        self.ensure_idle()?;
        if usize::from(source_id.is_some())
            + usize::from(code.is_some())
            + usize::from(text.is_some())
            != 1
        {
            return Err("Choose a file, text, or a receive code".into());
        }
        let settings = self.settings();
        settings.validate()?;
        let receiving = code.is_some();
        let options = settings.options(receiving)?;
        let mut config = self.config.lock().unwrap().clone();
        config.server_url =
            config::normalize_server_url(&settings.server).map_err(|e| e.to_string())?;
        hermod_core::session::expected_spki(&config).map_err(|e| e.to_string())?;
        let action = if let Some(code) = code {
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
        *self.job.lock().unwrap() = Some(Job::start(
            config,
            options,
            action,
            settings.words,
            Box::new(|| {}),
        ));
        Ok(())
    }
    pub fn transfer_destination(&self) -> Option<String> {
        self.transfer_destination.lock().unwrap().clone()
    }
    pub fn status(&self) -> Option<Status> {
        self.job.lock().unwrap().as_ref().map(|job| {
            let s = job.snapshot();
            Status {
                export_error: None,
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
                transport: s.transport,
                received_id: s.path.and_then(|p| {
                    p.parent()?
                        .parent()?
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                }),
            }
        })
    }
    pub fn cancel(&self) {
        if let Some(job) = self.job.lock().unwrap().as_ref() {
            job.cancel();
        }
    }
    pub fn list_received(&self) -> Result<Vec<storage::Received>, String> {
        storage::list(&self.inbox)
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
        let url = config::normalize_server_url(server).map_err(|e| e.to_string())?;
        runtime().block_on(async {
            let fp = tokio::time::timeout(
                Duration::from_secs(15),
                hermod_core::signal::tls::fetch_server_spki(&url),
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
        let mut cfg = self.config.lock().unwrap();
        let mut updated = cfg.clone();
        updated.trusted_servers.insert(url, fingerprint);
        config::save_to(&updated, &self.root.join("config.yaml")).map_err(|e| e.to_string())?;
        *cfg = updated;
        Ok(())
    }
    pub fn logs(&self) -> Vec<String> {
        logs::read()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_persist_and_reject_invalid_or_unapproved_values() {
        let root = std::env::temp_dir().join(format!("hermod-settings-{}", storage::new_id()));
        let engine = Engine::new(root.clone(), Config::default()).unwrap();
        assert!(Engine::new(root.clone(), Config::default()).is_err());
        let mut settings = engine.settings();
        assert_eq!(settings.connection.limit_mbps, "8");
        assert_eq!(settings.language, "system");
        // 旧设置没有语言字段时仍可加载，并默认跟随系统。
        let mut legacy = serde_json::to_value(&settings).unwrap();
        legacy.as_object_mut().unwrap().remove("language");
        assert_eq!(
            serde_json::from_value::<Settings>(legacy).unwrap().language,
            "system"
        );
        settings.language = "zh-CN".into();
        settings.connection.limit_mbps = "6.5".into();
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
        assert_eq!(engine.settings().connection.limit_mbps, "6.5");
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
