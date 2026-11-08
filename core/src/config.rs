//! 配置：与 Go 版 schema 双向兼容的 YAML 配置（M1.5）。
//!
//! 对应 Go 版 `hermod-go/internal/config/config.go`（决策 D15）：
//! - `Config`（第 34 行）         → [`Config`]
//! - `TLSConfig`（第 28 行）      → [`TlsConfig`]
//! - `Default()`（第 43 行）      → [`Config::default`]
//! - `Load`/`Save`（第 78/94 行）  → [`load`]/[`save`]（及可测的 `*_from`/`*_to`）
//! - `NormalizeServerURL`（第 244 行）→ [`normalize_server_url`]
//!
//! 关键语义（与 Go 一致）：**加载 = 先取默认值，再用 YAML 里出现的字段覆盖**。
//! 所以 YAML 里没写的字段（比如整个 `tls_configuration` 都没写）会得到 Go
//! 默认值，而不是空值。Rust 侧用 serde 的**字段级 `default`** 复刻这一点。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// TLS 曲线 / 密码套件偏好（对应 Go `TLSConfig`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TlsConfig {
    #[serde(rename = "prefer_curves", default = "default_prefer_curves")]
    pub prefer_curves: Vec<String>,
    #[serde(rename = "cipher_suites", default = "default_cipher_suites")]
    pub cipher_suites: Vec<String>,
}

/// 顶层配置（对应 Go `Config`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// 旧配置迁移标记；新配置只保留当前服务器的信任记录。
    #[serde(default)]
    pub trust_version: u8,
    #[serde(rename = "server_url", default = "default_server_url")]
    pub server_url: String,
    #[serde(rename = "tls_configuration", default = "default_tls_config")]
    pub tls: TlsConfig,
    #[serde(
        rename = "server_cert_pem",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub server_cert_pem: String,
    #[serde(
        rename = "server_key_pem",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub server_key_pem: String,
    #[serde(rename = "trusted_servers", default = "default_trusted_servers")]
    pub trusted_servers: BTreeMap<String, String>,
}

/// 默认认证（信令）服务器。
pub const DEFAULT_SERVER_URL: &str = "wss://oracle.stariverfeel.eu.org:4376";

/// 默认服务器的 SPKI 指纹（SHA-256，hex）——内置信任，免去首次 `trust`。
pub const DEFAULT_SERVER_FINGERPRINT: &str =
    "9ed110b37cd318d8c2ade01cf0b128a72c986ae051bd72dce9c05cf7834891cd";

impl Default for Config {
    /// 应用默认值：默认 server 为 [`DEFAULT_SERVER_URL`]，并内置信任其指纹。
    fn default() -> Self {
        Config {
            trust_version: 1,
            server_url: default_server_url(),
            tls: default_tls_config(),
            server_cert_pem: String::new(),
            server_key_pem: String::new(),
            trusted_servers: default_trusted_servers(),
        }
    }
}

impl Config {
    /// 指纹验证完成后一次性替换服务器和信任，保留旧 YAML 字段以便升级。
    pub fn replace_trust(&mut self, server: &str, fingerprint: &str) -> Result<(), ConfigError> {
        let url = normalize_server_url(server)?;
        if fingerprint.len() != 64 || !fingerprint.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ConfigError::InvalidFingerprint);
        }
        self.server_url = url.clone();
        self.trusted_servers = BTreeMap::from([(url, fingerprint.to_ascii_lowercase())]);
        self.trust_version = 1;
        Ok(())
    }

    /// 迁移时按实际选中的服务器收敛；缺少匹配指纹时保持未信任状态。
    pub fn retain_active_trust(&mut self) -> Result<(), ConfigError> {
        self.server_url = normalize_server_url(&self.server_url)?;
        let mut active = None;
        for (url, fingerprint) in &self.trusted_servers {
            if normalize_server_url(url).ok().as_ref() == Some(&self.server_url) {
                if active
                    .as_ref()
                    .is_some_and(|value: &String| !value.eq_ignore_ascii_case(fingerprint))
                {
                    return Err(ConfigError::InvalidFingerprint);
                }
                active = Some(fingerprint.clone());
            }
        }
        self.trusted_servers.clear();
        if let Some(fingerprint) = active {
            self.replace_trust(&self.server_url.clone(), &fingerprint)?;
        }
        self.trust_version = 1;
        Ok(())
    }
}

fn default_server_url() -> String {
    DEFAULT_SERVER_URL.to_string()
}

fn default_trusted_servers() -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    m.insert(
        DEFAULT_SERVER_URL.to_string(),
        DEFAULT_SERVER_FINGERPRINT.to_string(),
    );
    m
}

fn default_prefer_curves() -> Vec<String> {
    vec!["X25519MLKEM768".to_string()]
}

fn default_cipher_suites() -> Vec<String> {
    vec![
        "TLS_AES_256_GCM_SHA384".to_string(),
        "TLS_CHACHA20_POLY1305_SHA256".to_string(),
    ]
}

fn default_tls_config() -> TlsConfig {
    TlsConfig {
        prefer_curves: default_prefer_curves(),
        cipher_suites: default_cipher_suites(),
    }
}

/// 配置目录（Unix: `$HOME/.config/hermod`；Windows: `%AppData%\Hermod`）。
/// 拿不到 HOME 时回退到临时目录，避免把私钥写到不受控的工作目录（对应 Go `Dir`）。
pub fn dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            return PathBuf::from(appdata).join("Hermod");
        }
        std::env::temp_dir().join("Hermod")
    }
    #[cfg(not(windows))]
    {
        if let Some(directory) =
            std::env::var_os("XDG_CONFIG_HOME").filter(|d| Path::new(d).is_absolute())
        {
            return PathBuf::from(directory).join("hermod");
        }
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(".config").join("hermod");
        }
        std::env::temp_dir().join("hermod")
    }
}

/// 配置文件完整路径（`dir()/config.yaml`，对应 Go `Path`）。
pub fn path() -> PathBuf {
    dir().join("config.yaml")
}

/// 读配置；文件不存在 → 返回默认值（对应 Go `Load`）。
pub fn load() -> Result<Config, ConfigError> {
    load_from(&path())
}

/// 写配置：自动建目录、文件权限 0600（对应 Go `Save`）。
pub fn save(cfg: &Config) -> Result<(), ConfigError> {
    save_to(cfg, &path())
}

/// 从指定路径读（测试用）；文件不存在 → 默认值。
pub fn load_from(p: &Path) -> Result<Config, ConfigError> {
    load_selected(p, None)
}

/// GUI 的旧地址在 settings.json 中；必须先选择地址，再收敛旧信任列表。
pub fn load_selected(p: &Path, selected: Option<&str>) -> Result<Config, ConfigError> {
    if !p.exists() {
        return Ok(Config::default());
    }
    let data = std::fs::read_to_string(p).map_err(ConfigError::Read)?;
    let mut config: Config = serde_yaml::from_str(&data).map_err(ConfigError::Parse)?;
    if config.trust_version == 0
        && let Some(server) = selected
    {
        config.server_url = server.into();
    }
    config.retain_active_trust()?;
    Ok(config)
}

/// 写到指定路径（测试用）；权限 0600。
pub fn save_to(cfg: &Config, p: &Path) -> Result<(), ConfigError> {
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(ConfigError::Write)?;
    }
    let mut cfg = cfg.clone();
    cfg.retain_active_trust()?;
    let data = serde_yaml::to_string(&cfg).map_err(ConfigError::Serialize)?;
    // 独占临时文件并原子替换，失败不会破坏已信任的配置。
    use std::io::Write;
    let mut random = [0u8; 8];
    getrandom::fill(&mut random).map_err(|e| ConfigError::Write(std::io::Error::other(e)))?;
    let temporary = p.with_extension(format!("{}.tmp", crate::transfer::hex(&random)));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options.open(&temporary)?;
        file.write_all(data.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&temporary, p)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(ConfigError::Write)
}

/// 把 server URL 规范成 `scheme://host:port`（对应 Go `NormalizeServerURL`）：
///
/// - 无 scheme 时默认补 `wss://`；
/// - 丢弃 path / query / fragment；
/// - host 转小写；IPv6 加 `[]`；
/// - 无端口时默认 `4376`。
///
/// 例：`wss://Relay:4376/ws → wss://relay:4376`、`relay → wss://relay:4376`。
pub fn normalize_server_url(input: &str) -> Result<String, ConfigError> {
    if input.is_empty() {
        return Err(ConfigError::EmptyServerUrl);
    }
    // 与 Go 一致：只有 "wss://" / "ws://" 开头才不改写，其余一律补 wss://
    let s = if input.starts_with("wss://") || input.starts_with("ws://") {
        input.to_string()
    } else {
        format!("wss://{input}")
    };
    let (scheme, rest) = s.split_once("://").ok_or(ConfigError::InvalidServerUrl)?;
    // 丢弃 path / query / fragment，只留 authority（host[:port]）
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() {
        return Err(ConfigError::NoHost(input.to_string()));
    }
    let (host, port) = split_host_port(authority);
    if host.is_empty() {
        return Err(ConfigError::NoHost(input.to_string()));
    }
    let host = host.to_ascii_lowercase();
    // 无括号的 IPv6（如 "::1:4376"）需要补 []，对应 Go 的 net.JoinHostPort
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host
    };
    Ok(format!("{scheme}://{host}:{}", port.unwrap_or("4376")))
}

/// 拆 authority 为 (host, 可选 port)，支持 `[::1]:4376` 形式。
fn split_host_port(authority: &str) -> (&str, Option<&str>) {
    if let Some(rest) = authority.strip_prefix('[') {
        // 括号形式：要么 [::1]，要么 [::1]:4376
        return match rest.find(']') {
            Some(end) => {
                // authority = '[' + rest，所以 ']' 在 authority 里的下标是 end+1
                let host = &authority[..end + 2];
                let after = &rest[end + 1..];
                let port = after.strip_prefix(':').filter(|p| !p.is_empty());
                (host, port)
            }
            None => (authority, None),
        };
    }
    // 普通 host[:port]：仅当冒号后全是数字才当端口（避免 IPv6 无括号误判）
    match authority.rsplit_once(':') {
        Some((h, p)) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => (h, Some(p)),
        _ => (authority, None),
    }
}

/// 配置错误。
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Invalid or conflicting server fingerprint")]
    InvalidFingerprint,
    #[error("read config: {0}")]
    Read(#[from] std::io::Error),
    #[error("write config: {0}")]
    Write(#[source] std::io::Error),
    #[error("parse config: {0}")]
    Parse(#[source] serde_yaml::Error),
    #[error("serialize config: {0}")]
    Serialize(#[source] serde_yaml::Error),
    #[error("server URL is empty")]
    EmptyServerUrl,
    #[error("invalid server URL")]
    InvalidServerUrl,
    #[error("server URL {0} has no host component")]
    NoHost(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_and_replacement_keep_only_the_selected_trust() {
        let path = temp_path("single-trust");
        let mut config = Config {
            trust_version: 0,
            ..Config::default()
        };
        config
            .trusted_servers
            .insert("wss://custom:4376".into(), "ab".repeat(32));
        std::fs::write(&path, serde_yaml::to_string(&config).unwrap()).unwrap();
        let mut migrated = load_selected(&path, Some("custom")).unwrap();
        assert_eq!(migrated.server_url, "wss://custom:4376");
        assert_eq!(migrated.trusted_servers.len(), 1);
        let previous = migrated.clone();
        assert!(migrated.replace_trust("replacement", "invalid").is_err());
        assert_eq!(migrated, previous);
        migrated
            .replace_trust("replacement", &"CD".repeat(32))
            .unwrap();
        save_to(&migrated, &path).unwrap();
        // 新配置以原子保存的服务器为准，不再被旧 settings.json 的地址覆盖。
        let loaded = load_selected(&path, Some("custom")).unwrap();
        assert_eq!(loaded, migrated);
        assert_eq!(loaded.trusted_servers.len(), 1);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn saving_empty_trust_does_not_restore_the_builtin_fingerprint() {
        let path = temp_path("empty-trust");
        let config = Config {
            trusted_servers: BTreeMap::new(),
            ..Default::default()
        };
        save_to(&config, &path).unwrap();
        assert_eq!(load_from(&path).unwrap(), config);
        std::fs::remove_file(path).unwrap();
    }

    fn temp_path(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("hermod-config-{tag}-{n}.yaml"))
    }

    /// Go 版写出的配置（yaml.v3 风格）必须被本 Rust 版原样解析（M1.5 验收）。
    #[test]
    fn parses_go_produced_yaml() {
        let yaml = r#"
server_url: wss://relay.example.com:4376
tls_configuration:
  prefer_curves:
    - X25519MLKEM768
  cipher_suites:
    - TLS_AES_256_GCM_SHA384
    - TLS_CHACHA20_POLY1305_SHA256
server_cert_pem: "-----BEGIN CERTIFICATE-----\nabc\n-----END CERTIFICATE-----"
trusted_servers:
  wss://relay.example.com:4376: aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899
"#;
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.server_url, "wss://relay.example.com:4376");
        assert_eq!(cfg.tls.prefer_curves, vec!["X25519MLKEM768"]);
        assert_eq!(
            cfg.tls.cipher_suites,
            vec!["TLS_AES_256_GCM_SHA384", "TLS_CHACHA20_POLY1305_SHA256"]
        );
        assert!(
            cfg.server_cert_pem
                .starts_with("-----BEGIN CERTIFICATE-----")
        );
        assert_eq!(
            cfg.trusted_servers
                .get("wss://relay.example.com:4376")
                .unwrap(),
            "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899"
        );
    }

    /// YAML 里缺的字段得到 Go 默认值（load-onto-default 语义）。
    #[test]
    fn missing_fields_get_defaults() {
        let cfg: Config = serde_yaml::from_str("server_url: wss://other:9999\n").unwrap();
        assert_eq!(cfg.server_url, "wss://other:9999");
        assert_eq!(cfg.tls.prefer_curves, vec!["X25519MLKEM768"]); // 默认曲线保留
        assert_eq!(
            cfg.trusted_servers
                .get(DEFAULT_SERVER_URL)
                .map(String::as_str),
            Some(DEFAULT_SERVER_FINGERPRINT)
        );
        // 空 YAML → 全部默认
        let cfg: Config = serde_yaml::from_str("").unwrap();
        assert_eq!(cfg.server_url, DEFAULT_SERVER_URL);
    }

    /// 保存 → 读回 往返一致；文件不存在 → 默认值。
    #[test]
    fn save_load_roundtrip() {
        let p = temp_path("roundtrip");
        let cfg = Config {
            server_url: "wss://relay.example.com:4376".to_string(),
            trusted_servers: BTreeMap::from([(
                "wss://relay.example.com:4376".to_string(),
                "aa".repeat(32),
            )]),
            ..Config::default()
        };
        save_to(&cfg, &p).unwrap();
        let loaded = load_from(&p).unwrap();
        assert_eq!(loaded, cfg);
        std::fs::remove_file(&p).ok();

        // 不存在的文件 → 默认值
        let missing = temp_path("missing");
        assert_eq!(load_from(&missing).unwrap(), Config::default());
    }

    /// URL 规范化：Go 文档里的三个示例 + 边界。
    #[test]
    fn normalize_server_url_go_examples() {
        assert_eq!(
            normalize_server_url("wss://Relay:4376/ws").unwrap(),
            "wss://relay:4376"
        );
        assert_eq!(normalize_server_url("relay").unwrap(), "wss://relay:4376");
        assert_eq!(
            normalize_server_url("ws://relay:9090").unwrap(),
            "ws://relay:9090"
        );
        assert!(normalize_server_url("").is_err());
        assert!(normalize_server_url("wss:///nohost").is_err());
        assert_eq!(normalize_server_url("[::1]").unwrap(), "wss://[::1]:4376");
        assert_eq!(
            normalize_server_url("wss://[::1]:8080/path?x=1").unwrap(),
            "wss://[::1]:8080"
        );
    }
}
