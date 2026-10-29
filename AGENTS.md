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
smoltcp TCP (CLI sender default, CUBIC); QUIC uses quinn/TLS 1.3 (GUI sender default, BBR).
The receiver defaults to Auto. `AnyTransport` / `AnyConnection` dispatch at runtime;
neither transport depends on the other. `--mtu` and `--congestion` are interpreted and
validated for the selected protocol; no unsupported option is silently ignored.
Automatic binding uses a dual-stack socket with IPv4 fallback; internal overrides remain for tests.
Private/link-local IPv4 LAN and IPv6 candidates are exchanged through authenticated signaling.
There is no offline discovery, Internet IPv4 NAT traversal or relay. CPace and internal
SHA-256 integrity verification are retained. File offers are optional encrypted endpoint extensions.
GUI defaults to unlimited bandwidth. Each client profile persists one selected server and SPKI pin.

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
GUIs. A non-loopback private IPv4 or IPv6 address is needed; APP_BUILD.md shows an isolated namespace.
The ignored QUIC throughput benchmark is opt-in:
`cargo test -p hermod-core --release -- --ignored quic_throughput`.
No public signaling service is required for the quality gate.

Browser tests validate Fluent desktop UI, language selection and persistence through mocked IPC.
Linux builds x64 AppImage plus x64/ARM64 musl CLI; ARM64 skips GUI dependencies and packaging.
Native Linux x64 AppImage smoke tests check real window creation. Windows CI validates Store identity,
the reserved display name HermodTransfer in both package and application fields,
version and en-US/zh-CN resources, then signs a temporary copy for native installation/launch.
Both tested architectures are combined into one unsigned MSIXBundle in the internal
`store-windows` Actions artifact. MSIX/MSIXBundle files are never GitHub Release assets.
Bundle validation checks both embedded packages and their architecture coverage.
No Windows signing secrets, certificates
or user installation scripts are used. Android CI only builds and signs the ARM64 APK, then checks its
signature, SDK versions, ABI and alignment. Android emulator, instrumentation and
Android UI tests are not run. API/toolchain versions are pinned in APP_BUILD.md and Gradle.
`ci.yml` checks branch pushes and PRs and exposes the same checks via `workflow_call`.
`build-release.yml` runs for version-tag pushes and manual dispatch, reuses CI at the same
commit, and gates all packaging on it. Only version-tag pushes publish, after all jobs
succeed and an exact four-file Release allowlist is enforced (three Linux programs and one APK).
After Release succeeds, tag runs submit the Windows bundle on a disposable Windows runner through the pinned official Store CLI,
using `AZURE_AD_TENANT_ID`, `AZURE_AD_APPLICATION_CLIENT_ID`, `AZURE_AD_APPLICATION_SECRET`,
`SELLER_ID` and `MS_STORE_APP_ID`. Validate identity/version and preserve existing Store drafts;
never print credentials or full submission JSON/SAS URLs. Manual runs only build artifacts.
