//! 真实进程参数契约；每个进程使用隔离配置，不访问用户配置。
use std::{
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};
#[cfg(windows)]
#[path = "../src/windows.rs"]
mod windows;
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Sandbox(PathBuf);
impl Sandbox {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "hermod-cli-args-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(path.join("hermod")).unwrap();
        Self(path)
    }
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_hermod"));
        command
            .env("APPDATA", &self.0)
            .env("XDG_CONFIG_HOME", &self.0)
            .env_remove("HERMOD_PORT")
            .env_remove("HERMOD_LISTEN");
        command
    }
    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }
    fn config(&self, text: &str) {
        std::fs::write(self.0.join("hermod/config.yaml"), text).unwrap();
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn usage(output: Output) -> String {
    assert_eq!(
        output.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("error:"), "{stderr}");
    stderr
}
#[test]
fn help_and_version_are_successful_stdout() {
    let sandbox = Sandbox::new();
    for args in [
        &["device", "--help"][..],
        &["device", "rename", "--help"][..],
        &["--help"][..],
        &["--version"],
        &["tx", "--help"],
        &["rx", "--help"],
        &["serve", "--help"],
        &["trust", "--help"],
    ] {
        let output = sandbox.run(args);
        assert!(output.status.success(), "{args:?}");
        assert!(output.stderr.is_empty());
        assert!(!output.stdout.is_empty());
    }
    let help = String::from_utf8(sandbox.run(&["--help"]).stdout).unwrap();
    assert!(help.contains("auto, bbr, cubic, newreno"), "{help}");
    assert!(help.contains("auto, lan, wan"), "{help}");
    assert!(!help.contains("compatible"));
    let help = String::from_utf8(sandbox.run(&["rx", "--help"]).stdout).unwrap();
    assert!(help.contains("--output"));
    assert!(help.contains("[default: .]"));
}
#[test]
fn invalid_arguments_fail_before_config_or_network() {
    let sandbox = Sandbox::new();
    sandbox.config("not: [valid yaml");
    for args in [
        vec!["-4", "tx", "text", "-6"],
        vec!["-6", "tx", "text", "-4"],
        vec!["-4", "-6", "tx", "text"],
        vec!["tx", "text", "-4", "-6"],
        vec!["--ipv4", "trust", "localhost", "--ipv6"],
        vec!["--ipv6", "serve", "--ipv4"],
        vec!["tx"],
        vec!["rx"],
        vec!["rx", "1-a-b-c", "--from", "Laptop"],
        vec!["rx", "--from", "Laptop", "--pair"],
        vec!["rx", "--pair"],
        vec!["device", "rename", "invalid-id", "Laptop"],
        vec!["device", "id", "--streams", "1"],
        vec!["tx", "text", "--congestion", "reno"],
        vec!["tx", "text", "--profile", "compatible"],
        vec!["tx", "text", "--streams", "9"],
        vec!["rx", "1-a-b-c", "--streams", "0"],
        vec!["tx", "text", "--mtu", "1199"],
        vec!["tx", "text", "--mtu", "1453"],
        vec!["tx", "text", "--chunk", "1MB"],
        vec!["tx", "text", "--chunk", "1.5MiB"],
        vec!["tx", "text", "--chunk", "255KiB"],
        vec!["tx", "text", "--chunk", "5MiB"],
        vec!["tx", "text", "--window", "7MiB"],
        vec!["tx", "text", "--window", "257MiB"],
        vec!["tx", "text", "--udp-buffer", "0"],
        vec!["tx", "text", "--udp-buffer", "65MiB"],
        vec!["tx", "text", "--slice-size", "3MiB"],
        vec!["tx", "text", "--slice-size", "65MiB"],
        vec!["tx", "text", "--prefetch", "3"],
        vec!["tx", "text", "--prefetch", "65"],
        vec!["tx", "text", "--keepalive", "61"],
        vec!["tx", "text", "--idle-timeout", "9"],
        vec!["tx", "text", "--idle-timeout", "301"],
        vec!["rx", "1-a-b-c", "--keepalive", "30", "--idle-timeout", "30"],
        vec!["serve", "--port", "host:4376"],
        vec!["serve", "--port", "65536"],
    ] {
        let output = sandbox.run(&args);
        assert_eq!(
            output.status.code(),
            Some(2),
            "args={args:?}; stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        usage(output);
    }
}
#[test]
fn removed_options_are_not_accepted() {
    let sandbox = Sandbox::new();
    for flag in [
        "--words",
        "--server",
        "--destination",
        "--transport",
        "--wg-mtu",
        "--quic-mtu",
        "--quic-cc",
        "--limit-mbps",
        "--listen",
    ] {
        usage(sandbox.run(&["tx", "text", flag, "3"]));
    }
    usage(sandbox.run(&["rx", "1-a-b-c", "--destination", "."]));
    usage(sandbox.run(&["rx", "1-a-b-c", "--server", "localhost"]));
    usage(sandbox.run(&["serve", "--listen", ":4376"]));
}
#[test]
fn serve_and_trust_reject_even_explicit_defaults() {
    let sandbox = Sandbox::new();
    for (flag, value) in [
        ("--mtu", "auto"),
        ("--congestion", "bbr"),
        ("--profile", "auto"),
        ("--streams", "auto"),
        ("--chunk", "1MiB"),
        ("--window", "64MiB"),
        ("--udp-buffer", "16MiB"),
        ("--slice-size", "4MiB"),
        ("--prefetch", "16"),
        ("--keepalive", "10"),
        ("--idle-timeout", "30"),
    ] {
        for args in [
            vec![flag, value, "serve"],
            vec!["serve", flag, value],
            vec![flag, value, "trust", "localhost"],
            vec!["trust", "localhost", flag, value],
        ] {
            let error = usage(sandbox.run(&args));
            assert!(error.contains("only valid with tx/rx"), "{error}");
        }
    }
}
#[test]
fn environment_port_is_validated_and_explicit_default_wins() {
    let sandbox = Sandbox::new();
    usage(
        sandbox
            .command()
            .env("HERMOD_PORT", "host:4376")
            .arg("serve")
            .output()
            .unwrap(),
    );
    // 显式默认端口必须覆盖无效环境；下一项错误来自证书目录，不能进入网络监听。
    std::fs::write(sandbox.0.join("hermod/server"), "not a directory").unwrap();
    for args in [vec!["serve", "--port", "4376"], vec!["serve", "-p", "0"]] {
        let output = sandbox
            .command()
            .env("HERMOD_PORT", "invalid")
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("HERMOD_PORT"));
    }
}
#[test]
fn device_commands_persist_identity_and_manage_local_remarks() {
    #[cfg(windows)]
    hermod_core::identity::install_storage_permissions(std::sync::Arc::new(
        windows::NativePermissions,
    ))
    .unwrap();
    let sandbox = Sandbox::new();
    let first = sandbox.run(&["device", "id"]);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let id = String::from_utf8(first.stdout).unwrap();
    assert_eq!(id.trim().len(), 32);
    assert!(first.stderr.is_empty());
    assert_eq!(sandbox.run(&["device", "id"]).stdout, id.as_bytes());
    assert!(sandbox.run(&["device", "list"]).stdout.is_empty());
    let peer_dir = sandbox.0.join("peer");
    let peer = hermod_core::identity::Identity::load_or_create(&peer_dir).unwrap();
    let contacts = hermod_core::contacts::Contacts::open(&sandbox.0.join("hermod")).unwrap();
    contacts
        .stage_pairing(
            peer.device_id(),
            peer.public_key(),
            "",
            "cli-pair",
            [19; 32],
        )
        .unwrap();
    contacts
        .confirm_pairing(peer.device_id(), "cli-pair")
        .unwrap();
    let peer_id = peer.device_id().to_string();
    let rename = sandbox.run(&["device", "rename", &peer_id, "我的笔记本"]);
    assert!(
        rename.status.success(),
        "{}",
        String::from_utf8_lossy(&rename.stderr)
    );
    assert!(rename.stdout.is_empty());
    let list = sandbox.run(&["device", "list"]);
    assert!(list.status.success());
    let list = String::from_utf8(list.stdout).unwrap();
    assert!(list.contains(&peer_id) && list.contains("我的笔记本"));
    assert!(!list.contains(&"13".repeat(32)));
    let remove = sandbox.run(&["device", "remove", &peer_id]);
    assert!(remove.status.success());
    assert!(sandbox.run(&["device", "list"]).stdout.is_empty());
    let bytes = std::fs::read(sandbox.0.join("hermod/device-state/device.json")).unwrap();
    std::fs::write(sandbox.0.join("hermod/device-state/device.json"), "broken").unwrap();
    assert_eq!(sandbox.run(&["device", "id"]).status.code(), Some(1));
    assert_eq!(
        std::fs::read(sandbox.0.join("hermod/device-state/device.json")).unwrap(),
        b"broken"
    );
    assert!(!bytes.is_empty());
}

#[test]
fn config_does_not_supply_tuning_or_defaults() {
    let sandbox = Sandbox::new();
    sandbox.config("trust_version: 1\nserver_url: 'ws://127.0.0.1:1'\ntrusted_servers: {}\ntransfer_tuning:\n  streams: 999\n  chunk: 0\n  keepalive_secs: 300\n  idle_timeout_secs: 0\n");
    for args in [
        vec!["tx", "text"],
        vec![
            "-P",
            "WAN",
            "tx",
            "text",
            "-c",
            "BBR",
            "--streams",
            "AUTO",
            "-m",
            "AUTO",
        ],
        vec![
            "tx",
            "text",
            "--chunk",
            "262144",
            "--keepalive",
            "0",
            "--idle-timeout",
            "10",
        ],
    ] {
        let output = sandbox.run(&args);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("trust"), "{error}");
        assert!(output.stdout.is_empty());
    }
}
