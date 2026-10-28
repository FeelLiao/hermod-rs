//! 使用真实 CLI 进程和 GUI 应用层进行互通，不依赖外部信令服务。
use futures_util::{SinkExt, StreamExt};
use hermod_app_lib::app::{Engine, storage};
use hermod_core::{
    config::Config, crypto::wordlist::EFF_SHORT_WORDLIST, signal::protocol::Message, transfer,
};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
};
use tokio_tungstenite::tungstenite::Message as Frame;

async fn relay() -> (Config, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut cfg = Config {
        server_url: format!("ws://{}", listener.local_addr().unwrap()),
        ..Default::default()
    };
    cfg.trusted_servers
        .insert(cfg.server_url.clone(), "00".repeat(32));
    let task = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut tx = tokio_tungstenite::accept_async(tcp).await.unwrap();
        let request: Message =
            serde_json::from_str(tx.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        tx.send(Frame::text(
            serde_json::to_string(&Message::ok(request.channel_id, b"{}".to_vec())).unwrap(),
        ))
        .await
        .unwrap();
        let (tcp, _) = listener.accept().await.unwrap();
        let mut rx = tokio_tungstenite::accept_async(tcp).await.unwrap();
        let join: Message =
            serde_json::from_str(rx.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(request.channel_id, join.channel_id);
        rx.send(Frame::text(
            serde_json::to_string(&Message::ok(join.channel_id, b"{}".to_vec())).unwrap(),
        ))
        .await
        .unwrap();
        tx.send(Frame::text(
            serde_json::to_string(&Message::ready(join.channel_id)).unwrap(),
        ))
        .await
        .unwrap();
        loop {
            tokio::select! {
                Some(Ok(frame))=tx.next()=> { if rx.send(frame).await.is_err() { break; } },
                Some(Ok(frame))=rx.next()=> { if tx.send(frame).await.is_err() { break; } },
                else=>break,
            }
        }
    });
    (cfg, task)
}
fn cli(config: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_hermod"));
    command
        .env("XDG_CONFIG_HOME", config)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
}
async fn complete(engine: &Engine) {
    loop {
        let status = engine.status().unwrap();
        assert!(status.error.is_none(), "{:?}", status.error);
        if status.done {
            assert!(status.digest.is_some());
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

// 按词表完整匹配；yo-yo 自带连字符，不能用分隔符数量推算生成词数。
fn generated_word_count(code: &str) -> Option<usize> {
    let (channel, mut remaining) = code.split_once('-')?;
    channel.parse::<u16>().ok()?;
    let mut count = 0;
    loop {
        let suffix = EFF_SHORT_WORDLIST.iter().find_map(|word| {
            remaining
                .strip_prefix(word)
                .filter(|suffix| suffix.is_empty() || suffix.starts_with('-'))
        })?;
        count += 1;
        if suffix.is_empty() {
            return Some(count);
        }
        remaining = suffix.strip_prefix('-')?;
    }
}

#[test]
fn generated_word_count_handles_every_dictionary_word() {
    for word in EFF_SHORT_WORDLIST {
        let code = format!("42-acid-{word}-acorn-acre-acts");
        assert_eq!(generated_word_count(&code), Some(5), "{code}");
    }
    for (code, expected) in [
        ("42-yo-yo-acid-acorn-acre-acts", 5),
        ("42-acid-acorn-acre-acts-yo-yo", 5),
        ("42-yo-yo-yo-yo-yo-yo-yoyo-yo-yo", 5),
        ("42-acid-acorn-acre", 3),
        ("42-acid-acorn-acre-acts-afar-affix", 6),
    ] {
        assert_eq!(generated_word_count(code), Some(expected), "{code}");
    }
}

#[test]
fn generated_word_count_rejects_invalid_codes() {
    for code in [
        "invalid-acid-acorn-acre",
        "65536-acid-acorn-acre",
        "42-acid-unknown-acorn",
        "42-acidacorn-acre-acts",
        "42-acid--acorn-acre",
        "42-acid-acorn-acre-",
        "42-acid-yo-acorn",
        "42-",
        "42",
    ] {
        assert_eq!(generated_word_count(code), None, "{code}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cli_and_gui_interoperate_both_directions_with_files_and_text() {
    tokio::time::timeout(Duration::from_secs(180), async {
        for protocol in ["wireguard", "quic"] {
            for mode in ["gui-to-cli", "cli-to-gui", "cli-to-cli"] {
                for source in ["text", "file", "stdin"] {
                    if source == "stdin" && mode == "gui-to-cli" {
                        continue;
                    }
                    let text = source == "text";
                    let root =
                        std::env::temp_dir().join(format!("hermod-interop-{}", storage::new_id()));
                    let cfgdir = root.join("config");
                    std::fs::create_dir_all(cfgdir.join("hermod")).unwrap();
                    let (cfg, server) = relay().await;
                    hermod_core::config::save_to(&cfg, &cfgdir.join("hermod/config.yaml")).unwrap();
                    let engine = Engine::new(root.join("app"), cfg).unwrap();
                    let mut settings = engine.settings();
                    settings.connection.transport = protocol.into();
                    settings.connection.limit_mbps = "0.8".into();
                    settings.words = 5;
                    engine.save_settings(settings).unwrap();
                    engine
                        .set_destination(Some("original-folder".into()))
                        .unwrap();
                    let bytes = if text {
                        "Hello 世界 from Hermod".as_bytes().to_vec()
                    } else {
                        vec![0x51; 32768]
                    };
                    let input = root.join("source.bin");
                    std::fs::write(&input, &bytes).unwrap();
                    if mode == "gui-to-cli" {
                        if text {
                            engine
                                .start(None, None, Some(String::from_utf8(bytes.clone()).unwrap()))
                                .unwrap();
                        } else {
                            let source = engine.select_file(&input).unwrap();
                            engine.start(Some(source.id), None, None).unwrap();
                        }
                        let code = loop {
                            let state = engine.status().unwrap();
                            assert!(state.error.is_none(), "{:?}", state.error);
                            if let Some(code) = state.code {
                                break code;
                            }
                            tokio::time::sleep(Duration::from_millis(20)).await;
                        };
                        assert_eq!(generated_word_count(&code), Some(5), "{code}");
                        let mut changed = engine.settings();
                        changed.connection.transport = if protocol == "wireguard" {
                            "quic"
                        } else {
                            "wireguard"
                        }
                        .into();
                        engine.save_settings(changed).unwrap();
                        engine.set_destination(Some("next-folder".into())).unwrap();
                        assert_eq!(
                            engine.transfer_destination().as_deref(),
                            Some("original-folder")
                        );
                        let destination = root.join("cli-inbox");
                        let output = cli(&cfgdir)
                            .arg("rx")
                            .arg(code)
                            .arg("--destination")
                            .arg(&destination)
                            .output()
                            .await
                            .unwrap();
                        assert!(
                            output.status.success(),
                            "{}",
                            String::from_utf8_lossy(&output.stderr)
                        );
                        complete(&engine).await;
                        assert_eq!(
                            engine.status().unwrap().transport.as_deref(),
                            Some(protocol)
                        );
                        let received = std::fs::read_dir(&destination)
                            .unwrap()
                            .next()
                            .unwrap()
                            .unwrap()
                            .path();
                        assert_eq!(std::fs::read(received).unwrap(), bytes);
                    } else {
                        let input_arg = if source == "stdin" {
                            "-".into()
                        } else if text {
                            String::from_utf8(bytes.clone()).unwrap()
                        } else {
                            input.to_str().unwrap().into()
                        };
                        let mut child = cli(&cfgdir)
                            .stdin(if source == "stdin" {
                                Stdio::piped()
                            } else {
                                Stdio::null()
                            })
                            .args([
                                "tx",
                                "--transport",
                                protocol,
                                "--limit-mbps",
                                "0.8",
                                &input_arg,
                            ])
                            .spawn()
                            .unwrap();
                        if source == "stdin" {
                            let mut stdin = child.stdin.take().unwrap();
                            stdin.write_all(&bytes).await.unwrap();
                            stdin.shutdown().await.unwrap();
                            drop(stdin);
                        }
                        let mut lines = BufReader::new(child.stderr.take().unwrap()).lines();
                        let code = loop {
                            let line = lines
                                .next_line()
                                .await
                                .unwrap()
                                .expect("CLI did not produce a code");
                            if let Some(code) = line.strip_prefix("Transfer code: ") {
                                break code.to_string();
                            }
                        };
                        if mode == "cli-to-gui" {
                            engine.start(None, Some(code), None).unwrap();
                            complete(&engine).await;
                            let received = engine.list_received().unwrap();
                            assert_eq!(received.len(), 1);
                            assert_eq!(received[0].sha256, transfer::hash_bytes(&bytes));
                            let mut copy = Vec::new();
                            engine.export(&received[0].id, &mut copy).unwrap();
                            assert_eq!(copy, bytes);
                        } else {
                            let destination = root.join("cli-inbox");
                            let output = cli(&cfgdir)
                                .arg("rx")
                                .arg(code)
                                .arg("--destination")
                                .arg(&destination)
                                .output()
                                .await
                                .unwrap();
                            assert!(
                                output.status.success(),
                                "{}",
                                String::from_utf8_lossy(&output.stderr)
                            );
                            let received = std::fs::read_dir(destination)
                                .unwrap()
                                .next()
                                .unwrap()
                                .unwrap()
                                .path();
                            assert_eq!(std::fs::read(received).unwrap(), bytes);
                        }
                        let output = child.wait_with_output().await.unwrap();
                        assert!(output.status.success());
                    }
                    server.await.unwrap();
                    drop(engine);
                    std::fs::remove_dir_all(root).unwrap();
                }
            }
        }
    })
    .await
    .expect("CLI/GUI interoperability timed out");
}
