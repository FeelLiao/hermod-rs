# Agent Guide

Guidance for LLM agents working on this repository.

## Read first

- `README.md` / `README.zh-CN.md`: users, platforms, protocol defaults and migration.
- `CHANGELOG.md`: recent and historical changes.
- `APP_BUILD.md`: toolchains, fixed signing, CI gates and release allowlist.
- Module `//!` docs in `hermod-core/` and `hermod-app/src-tauri/src/app/`.

## Conventions

- Code identifiers and technical logs are English; GUI text supports English and Simplified Chinese.
  Use the shared catalog in `hermod-app/src/locales/zh-CN.json`; code comments are Chinese (zh-CN).
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

Browser tests validate Fluent desktop UI, language selection and persistence through mocked IPC.
Native Linux AppImage smoke tests check real window creation. Windows CI validates Store identity,
the reserved display name HermodTransfer in both package and application fields,
version and en-US/zh-CN resources, then signs a temporary copy for native installation/launch.
Both tested architectures are combined into one unsigned MSIXBundle; only the bundle is
released for Partner Center. Bundle validation checks both embedded packages and their architecture coverage.
No Windows signing secrets, certificates
or user installation scripts are used. Android CI only builds and signs the ARM64 APK, then checks its
signature, SDK versions, ABI and alignment. Android emulator, instrumentation and
Android UI tests are not run. API/toolchain versions are pinned in APP_BUILD.md and Gradle.
`ci.yml` checks branch pushes and PRs and exposes the same checks via `workflow_call`.
`build-release.yml` runs for version-tag pushes and manual dispatch, reuses CI at the same
commit, and gates all packaging on it. Only version-tag pushes publish, after all jobs
succeed and an exact Release file allowlist is enforced. Manual runs only build artifacts.
