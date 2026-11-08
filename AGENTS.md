# Agent Guide

Guidance for LLM agents working on this repository.

## Read first

- `README.md` / `README.zh-CN.md`: users, platforms, protocol defaults and migration.
- `CHANGELOG.md`: recent and historical changes.
- `APP_BUILD.md`: Flutter toolchains, Store identity, MSIX and CI gates.
- Module `//!` docs in `core/` (protocol and transports) and `flutter-bridge/`
  (`src/app/` application layer and `src/gui_api/` stable facade).

## Conventions

- Code identifiers and technical logs are English; GUI text supports English and Simplified Chinese. Code comments are Chinese (zh-CN).
- Commits use Conventional Commits: `<type>(<scope>): <summary>`.
- Handwritten unsafe Rust is forbidden by workspace lints.
- Never regenerate release signing keys silently or publish private keys.
- Preserve user inbox data. Cleanup removes incomplete files only after acquiring the storage lock.
- Use FVM commands for Flutter: `fvm flutter ...` and `fvm dart ...`.

Required quality gates:

```bash
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cd flutter
fvm dart format --output=none --set-exit-if-changed lib test
fvm flutter analyze
fvm flutter test
```

## Architecture

`core` contains cryptography, signaling with SPKI pinning, connection options, independent WireGuard and QUIC transports, transport-internal HTTP, session negotiation and tx/rx orchestration — the shared lower layer for CLI and GUI. The receiver defaults to Auto. GUI code must not implement transfer state, trust, hashing, inbox storage, timeouts, QUIC or WireGuard.

`hermod_flutter_bridge::app` owns settings, private verified inbox, sources, jobs and cancellation. `hermod_flutter_bridge::gui_api` is the only GUI business facade and never exposes private inbox paths.

`flutter` is the Material 3 Windows/Android presentation project. On Windows it loads `hermod_flutter_bridge.dll` through flutter_rust_bridge (FFI); the bridge exposes `gui_api` and is the only crate where the generated grpc-style glue contains unsafe. Handwritten unsafe remains forbidden everywhere; `core` and `cli` keep the workspace `forbid(unsafe_code)` lint. Windows uses Flutter's supported Win32 embedder, native file pickers and Store MSIX packaging. Add Windows App SDK functionality selectively without replacing the Flutter host.

`cli` is the terminal shell. No Tauri, React or WebView frontend remains.

## Tests and releases

Crypto tests use independent RFC vectors. Transport tests exercise handshake, HTTP, streams, digest verdict and cancellation. A private IPv4 or IPv6 test address is required for complete integration tests.

Windows CI builds native x64 and ARM64 Flutter applications and Rust bridges, packages one unsigned MSIX per architecture and combines them into `hermod-windows.msixbundle`. Validate Store identity `FeelLiao.HermodTransfer`, publisher, reserved display name `HermodTransfer`, version, en-US/zh-CN resources, both executable architectures and absence of signatures or secrets. Microsoft Store signs uploaded packages.

Linux releases contain only x64/ARM64 static musl CLI binaries. MSIX/MSIXBundle files remain Store-only artifacts and are never GitHub Release assets. Store submission must not print credentials, full submission JSON or SAS URLs.
