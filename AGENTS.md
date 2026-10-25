# Agent Guide

Guidance for LLM agents working on this repository.

## Read first

- `README.md` / `README.zh-CN.md`: users, platforms, protocol defaults and migration.
- `CHANGELOG.md`: recent and historical changes.
- `APP_BUILD.md`: toolchains, fixed signing, CI gates and release allowlist.
- Module `//!` docs in `hermod-core/` and `hermod-app/src-tauri/src/app/`.

## Conventions

- Code identifiers, logs and application UI are English; code comments are Chinese (zh-CN).
- Commits use Conventional Commits: `<type>(<scope>): <summary>`.
- Handwritten unsafe Rust is forbidden by workspace lints. Mobile file descriptors use the official Tauri fs plugin.
- Never regenerate release signing keys silently or publish private keys.
- Preserve user inbox data. Cleanup removes incomplete files only after acquiring the storage lock.
- Local verification is Linux-only for this release; other platform builds run in Actions.

Required quality gates:

```bash
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
npm ci --prefix hermod-app
npm run build --prefix hermod-app
npm test --prefix hermod-app
```

## Architecture

`hermod-core` contains crypto, the WebSocket signaling client with SPKI pinning,
shared connection options, independent WireGuard and QUIC transports, transport-internal
HTTP/1.1, session negotiation and tx/rx orchestration. WireGuard uses boringtun and
smoltcp TCP (default sender, CUBIC); QUIC uses quinn/TLS 1.3 (optional sender, BBR).
The receiver defaults to Auto. `AnyTransport` / `AnyConnection` dispatch at runtime;
neither transport depends on the other. `--mtu` and `--congestion` are interpreted and
validated for the selected protocol; no unsupported option is silently ignored.
Automatic binding is `[::]:0`; internal overrides remain for tests. IPv6-only behavior,
CPace and internal SHA-256 integrity verification are retained.

`hermod_app_lib::app::Engine` owns persistent settings, private verified inbox, sources,
job snapshots and cancellation. Every transfer copies settings before starting.
`hermod-cli` is a clap shell. Tauri exposes narrowly scoped commands
and native dialogs to shared React pages. The UI adapter selects official Fluent UI v9
on desktop and official Material Web on Android. The Android plugin under hermod-app
provides SAF, permissions, system color and a foreground service. Rust monitors jobs
and exports independently of WebView timers.
No frontend receives unrestricted private file paths. File streams are bounded;
unknown-size sources are staged privately and verified receive data survives export failures.

## Tests and releases

Crypto tests use independent RFC vectors (7748, 9380, 5869). Local transport tests
exercise handshake, streams, HTTP and digest verdict on both protocols. CLI integration
tests spawn actual command-line binaries and use the same application engine as both
GUIs. A non-loopback local IPv6 address is needed; APP_BUILD.md shows an isolated namespace.
The ignored QUIC throughput benchmark is opt-in:
`cargo test -p hermod-core --release -- --ignored quic_throughput`.
No public signaling service is required for the quality gate.

Browser tests validate Fluent desktop and Material phone/tablet UIs through mocked IPC. Native Linux AppImage smoke tests
check real window creation. Windows CI validates signing identity and native MSIX
installation/launch. Android CI tests API 34 and 37 with an unpublished x86_64 ABI;
only signed ARM64 APKs ship. API/toolchain versions are pinned in APP_BUILD.md and Gradle.
The workflow gates publishing on all jobs, then enforces an exact Release file allowlist.
