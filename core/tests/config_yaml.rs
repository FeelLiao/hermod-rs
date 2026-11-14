//! YAML 文件边界回归：保留 Go schema、PEM 内容及单服务器信任规则。
use hermod_core::config::{
    self, Config, ConfigError, DEFAULT_SERVER_FINGERPRINT, DEFAULT_SERVER_URL,
};
use std::{collections::BTreeMap, fs, path::PathBuf};

struct ConfigFile(PathBuf);

impl ConfigFile {
    fn new() -> Self {
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let name: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let directory = std::env::temp_dir().join(format!("hermod-config-yaml-{name}"));
        fs::create_dir(&directory).unwrap();
        Self(directory.join("config.yaml"))
    }

    fn write(&self, yaml: &str) {
        fs::write(&self.0, yaml).unwrap();
    }
}

impl Drop for ConfigFile {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(self.0.parent().unwrap());
    }
}

#[test]
fn missing_empty_and_null_files_use_defaults() {
    let file = ConfigFile::new();
    assert_eq!(config::load_from(&file.0).unwrap(), Config::default());
    for yaml in [
        "",
        " \n  \n",
        "# Go config\n",
        "---\n# empty\n...\n",
        "null\n",
        "~\n",
        "{}\n",
    ] {
        file.write(yaml);
        assert_eq!(
            config::load_from(&file.0).unwrap(),
            Config::default(),
            "{yaml:?}"
        );
    }
}

#[test]
fn go_yaml_pem_ipv6_roundtrip_accepts_unknown_fields() {
    let file = ConfigFile::new();
    let fingerprint = "ab".repeat(32);
    file.write(&format!(
        r#"# Go yaml.v3-style configuration
server_url: 'wss://[2001:db8::1]:4376/ws?source=go'
tls_configuration:
  prefer_curves:
    - X25519MLKEM768
    - X25519
  cipher_suites:
    - TLS_AES_256_GCM_SHA384
  future_tls_option: {{enabled: true}}
server_cert_pem: |
  -----BEGIN CERTIFICATE-----
  YWJj
  -----END CERTIFICATE-----
server_key_pem: |-
  -----BEGIN PRIVATE KEY-----
  ZGVm
  -----END PRIVATE KEY-----
trusted_servers:
  "wss://[2001:db8::1]:4376": "{fingerprint}"
  wss://inactive.example:4376: "{fingerprint}"
future_option:
  nested: [true, null, {{name: 'value: # not a comment'}}]
"#
    ));
    let loaded = config::load_from(&file.0).unwrap();
    assert_eq!(loaded.server_url, "wss://[2001:db8::1]:4376");
    assert_eq!(loaded.trust_version, 1);
    assert_eq!(loaded.tls.prefer_curves, ["X25519MLKEM768", "X25519"]);
    assert_eq!(loaded.tls.cipher_suites, ["TLS_AES_256_GCM_SHA384"]);
    assert_eq!(
        loaded.server_cert_pem,
        "-----BEGIN CERTIFICATE-----\nYWJj\n-----END CERTIFICATE-----\n"
    );
    assert_eq!(
        loaded.server_key_pem,
        "-----BEGIN PRIVATE KEY-----\nZGVm\n-----END PRIVATE KEY-----"
    );
    assert_eq!(
        loaded.trusted_servers,
        BTreeMap::from([(loaded.server_url.clone(), fingerprint)])
    );

    // Unknown fields remain accepted/ignored, as before; only known schema is saved.
    config::save_to(&loaded, &file.0).unwrap();
    assert_eq!(config::load_from(&file.0).unwrap(), loaded);
    // Save again to exercise replacement of an existing file, not just first creation.
    config::save_to(&loaded, &file.0).unwrap();
    assert_eq!(config::load_from(&file.0).unwrap(), loaded);
    let saved = fs::read_to_string(&file.0).unwrap();
    let decoded: Config = serde_saphyr::from_str(&saved).unwrap();
    assert_eq!(decoded, loaded);
    assert!(!saved.contains("future_option"));
}

#[test]
fn partial_tls_fields_keep_go_defaults_and_empty_trust_stays_empty() {
    let file = ConfigFile::new();
    file.write("tls_configuration:\n  prefer_curves: [X25519]\ntrusted_servers: {}\n");
    let loaded = config::load_from(&file.0).unwrap();
    assert_eq!(loaded.tls.prefer_curves, ["X25519"]);
    assert_eq!(
        loaded.tls.cipher_suites,
        Config::default().tls.cipher_suites
    );
    assert!(loaded.trusted_servers.is_empty());
    config::save_to(&loaded, &file.0).unwrap();
    assert_eq!(config::load_from(&file.0).unwrap(), loaded);

    file.write("tls_configuration: {}\n");
    assert_eq!(config::load_from(&file.0).unwrap(), Config::default());
}

#[test]
fn malformed_wrong_type_and_multiple_documents_are_errors_not_defaults() {
    let file = ConfigFile::new();
    for yaml in [
        "server_url: [unterminated\n",
        "server_url: [not, a, string]\n",
        "tls_configuration: wrong-type\n",
        "trusted_servers: [not-a-map]\n",
        "trust_version: not-a-number\n",
        "future_option: [unterminated\n",
        "---\n{}\n---\n{}\n",
        "---\n# empty first\n---\n{}\n",
        "[]\n",
    ] {
        file.write(yaml);
        assert!(
            matches!(config::load_from(&file.0), Err(ConfigError::Parse(_))),
            "{yaml:?}"
        );
        assert_eq!(fs::read_to_string(&file.0).unwrap(), yaml);
    }
}

#[test]
fn legacy_selection_retains_only_matching_trust_and_new_version_ignores_selection() {
    let file = ConfigFile::new();
    let fingerprint = "CD".repeat(32);
    file.write(&format!("server_url: {DEFAULT_SERVER_URL}\ntrusted_servers:\n  '{DEFAULT_SERVER_URL}': '{DEFAULT_SERVER_FINGERPRINT}'\n  'wss://[::1]:4376': '{fingerprint}'\n"));
    let selected = config::load_selected(&file.0, Some("[::1]")).unwrap();
    assert_eq!(selected.server_url, "wss://[::1]:4376");
    assert_eq!(
        selected.trusted_servers,
        BTreeMap::from([(
            selected.server_url.clone(),
            fingerprint.to_ascii_lowercase()
        )])
    );
    config::save_to(&selected, &file.0).unwrap();
    assert_eq!(
        config::load_selected(&file.0, Some("other")).unwrap(),
        selected
    );

    // Selecting a server without a matching fingerprint must not trust a default/old server.
    file.write("server_url: untrusted.example\n");
    let untrusted = config::load_from(&file.0).unwrap();
    assert!(untrusted.trusted_servers.is_empty());
    config::save_to(&untrusted, &file.0).unwrap();
    assert_eq!(config::load_from(&file.0).unwrap(), untrusted);
}

#[test]
fn conflicting_or_invalid_active_fingerprints_fail_without_overwriting_file() {
    let file = ConfigFile::new();
    file.write(&format!(
        "server_url: relay\ntrusted_servers:\n  relay: '{}'\n  'wss://relay:4376': '{}'\n",
        "ab".repeat(32),
        "cd".repeat(32)
    ));
    assert!(matches!(
        config::load_from(&file.0),
        Err(ConfigError::InvalidFingerprint)
    ));
    let previous = fs::read(&file.0).unwrap();
    let invalid = Config {
        server_url: "wss://relay:4376".into(),
        trusted_servers: BTreeMap::from([("wss://relay:4376".into(), "invalid".into())]),
        ..Config::default()
    };
    assert!(matches!(
        config::save_to(&invalid, &file.0),
        Err(ConfigError::InvalidFingerprint)
    ));
    assert_eq!(fs::read(&file.0).unwrap(), previous);
}

#[cfg(unix)]
#[test]
fn saved_config_remains_private() {
    use std::os::unix::fs::PermissionsExt;
    let file = ConfigFile::new();
    config::save_to(&Config::default(), &file.0).unwrap();
    assert_eq!(
        fs::metadata(&file.0).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
