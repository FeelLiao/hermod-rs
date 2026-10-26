# Changelog

All notable changes to this project are documented in this file. The format is
based on [Keep a Changelog](https://keepachangelog.com/), and this project
adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Bilingual Microsoft Store listing copy, six desktop screenshots per language,
  app icons, promotional art and a field-by-field upload guide. A Linux capture
  script regenerates and validates the assets using the shared UI and demo data.

## [0.4.3] - 2026-10-26

### Added

- English/Simplified Chinese GUI with system-language detection, a persistent language
  setting, translated accessibility labels, transfer states, native dialogs and Android
  foreground notifications. Both platforms share one translation catalog.
- English/Chinese MSIX declarations and localized descriptions compiled into resources.pri.

### Changed

- Windows MSIX packages now use the Partner Center identity `FeelLiao.HermodTransfer`,
  publisher `CN=37A01BFF-1DAB-45E5-9B67-AA76C0655A7C` and display publisher `FeelLiao`.
  Release MSIX files are unsigned Store uploads; CI signs only a disposable copy for
  installation/launch checks and verifies the expected package family name.
- Map app versions to Store versions as `(major + 1).minor.patch.0` (0.4.3 → 1.4.3.0),
  satisfying the nonzero major and reserved revision requirements without future collisions.
- Release allowlist contains seven packages/programs. Bump app versions to 0.4.3 and
  Android versionCode to 40300; existing settings default to system language.

### Removed

- Windows release signing secrets, test certificate exports and `Install-Hermod.ps1`.
  The one-time signing initializer now creates only the fixed Android key; existing
  local signing files are preserved. Windows users install through Store after publication.

## [0.4.2] - 2026-10-25

### Changed

- Split branch/PR quality checks into `ci.yml` and tag/manual packaging into
  `build-release.yml`. Packaging reuses CI at the same commit and waits for it;
  only a version-tag push publishes a Release. Manual runs only build artifacts.
- Bump workspace, frontend and Tauri versions to 0.4.2 and Android versionCode to 40200.

### Fixed

- Count complete dictionary words in CLI/GUI interoperability checks. Random codes
  containing `yo-yo` previously caused a false word-count failure in CI. Add
  deterministic coverage for the full wordlist and hyphenated-code password round trips;
  the transfer-code format and protocol behavior remain unchanged.

## [0.4.1] - 2026-10-25

### Changed

- Android CI builds and signs only the ARM64 APK, retaining signature, SDK, ABI
  and 16 KB alignment checks. Release publishing no longer waits for emulator jobs.
- Bump workspace, frontend and Tauri versions to 0.4.1 and Android versionCode to 40100.

### Removed

- Android API 34/37 emulator jobs, x86_64 test APK builds, instrumentation fixtures,
  test dependencies, test-copy generation steps and the separate Android lint task.
- Android phone/tablet browser test projects; desktop browser and Rust tests remain.

### Fixed

- Declare the Android document plugin's Activity dependency directly so its
  `ActivityResult` callbacks compile; app-module dependencies do not supply the
  plugin's compile classpath.
- Check fixed Windows and Android release signing secrets before installing build
  tools or compiling clients, and report the missing secret names without values.
- Document the signing-secret file mapping and how to retry a corrected workflow.

## [0.4.0] - 2026-10-25

### Added

- Shared Rust application engine and validated settings, persistent GUI configuration,
  bounded document streams, verified inbox exports and in-memory logs.
- Tauri Android Material Design 3 Web GUI sharing React pages with desktop; SAF URI
  access, dynamic colors, foreground progress and notification cancellation.
  Minimum API 34; compile/target API 37, with pinned SDK/Gradle/AGP/NDK toolchains.
- Official Fluent UI React v9 desktop interface with system themes, file/text sending,
  receive, cancellation, advanced network options and fingerprint trust management.
- Fixed signing-key initialization and documented GitHub Secrets, PowerShell-only
  Windows installation, MSIX installation/launch checks and Android API 34/37 tests.
- English and Chinese quick-start documentation, settings/export tests and actual
  CLI-to-GUI engine interoperability checks on both protocols.

### Changed

- Sender default is WireGuard; receiver remains Auto. Unified `--mtu` and
  `--congestion` apply to the selected protocol and reject incompatible settings.
- WireGuard auto MTU is 1400 and congestion control is CUBIC; QUIC auto uses BBR,
  begins MTU discovery at 1200 and obeys the configured UDP payload ceiling.
- GUI starts at 8 Mbps, CLI remains unlimited; saved changes affect future jobs.
- Release matrix: Windows GUI x64/ARM64 MSIX, Linux GUI x64/ARM64 AppImage,
  Linux CLI x64/ARM64 musl, Android ARM64 APK. Exact attachment allowlist, gated
  publication, fixed Windows PFX and Android keystore, no generated replacement keys.
- CLI respects an absolute XDG_CONFIG_HOME, falling back to ~/.config/hermod.

### Removed

- Public bind/listen controls, protocol-specific MTU/CC flags and unimplemented serve.
- macOS releases, Windows/Android CLI releases, portable Windows ZIP tooling,
  obsolete native Android UI/bridge crates and the separate GUI engine crate.
- Root packaging/windows nesting: GUI packaging, templates and native adapters now
  live entirely under hermod-app.
- Release checksums and auxiliary attachments; internal transfer SHA-256 remains.


## [0.3.1] - 2026-10-25

### Changed

- Windows GUI releases now contain only a portable ZIP; Linux GUI releases only
  AppImage. Android, macOS and the existing CLI release artifacts are preserved.
- Windows ZIP includes runtime instructions and optional tools to create an
  unsigned Microsoft Store MSIX using the publisher's own Partner Center identity.
  Ordinary ZIP files cannot be submitted directly as Store app packages.
- Platform-specific Tauri defaults disable Windows installers and keep macOS
  app/DMG bundles while limiting Linux to AppImage.

### Fixed

- Windows releases no longer depend on ephemeral MSIX certificates or the failing
  self-signed certificate-chain verification step. Manual Store packaging does
  not sign packages or change certificate trust stores.

## [0.3.0] - 2026-10-25

### Added

- Tauri 2 app for Windows, Linux, macOS and Android, replacing the egui prototype.
  Material Design 3 with desktop/phone layouts, dark mode, progress/RTT, transfer
  codes, cancellation, private verified inbox, native selection and export.
- Android document URI streaming, unknown-length staging and a foreground transfer
  service; no broad storage permissions or file contents passed through JavaScript.
- Test-signed Windows MSIX and Android APK, Linux AppImage/deb and unnotarized
  macOS Intel/ARM apps/DMGs in the unified GitHub Release workflow. Existing three
  command-line outputs are preserved; all release assets receive SHA-256 checksums.
- Per-transfer `--limit-mbps` on both CLI commands and an editable desktop cap.
  Payload is paced before transport writes; upgraded senders honor the receiver's
  budget. Limited QUIC sessions use smaller application windows.
- An isolated IPv6 latency reproduction example.
- WSL mirrored-networking setup and upload queueing investigation documentation.

### Fixed

- Portable receive filenames handle Windows device names, forbidden characters,
  trailing dots/spaces and overlong Unicode names without overwriting files.
- QUIC HTTP cancellation test waits for payload progress instead of a fixed sleep,
  preventing cancellation during the handshake under build load.

- Transfer codes appear only after signaling channel allocation succeeds, and
  signaling connection setup has a 15-second timeout.
- WireGuard opens its HTTP listener before its handshake to allow asynchronous
  file opening and desktop scheduling without racing a fast receiver.
- Test temporary directories include process IDs to avoid stale-run collisions.

## [0.2.1] - 2026-10-20

### Changed

- The sender progress bar now shows the QUIC **wire rate** (`udp_tx.bytes`
  delta), which keeps moving while the local send buffer drains. The bar's
  position and percentage remain based on file bytes written; WireGuard keeps
  its existing local rate display.

## [0.2.0] - 2026-10-20

### Added

- **QUIC data plane** is now the default transport; WireGuard remains available
  with `--transport wireguard`.
- The receiver defaults to `--transport auto`: it reads a small transport header
  from the sender's AEAD-encrypted endpoint bundle and selects the matching
  implementation itself.
- QUIC transport with TLS 1.3 server-certificate pinning, per-session ephemeral
  certificates, endpoint authentication through the CPace-encrypted endpoint
  bundle, and a 32-byte session-fingerprint stream preamble plus a one-time HTTP
  token for client authorization.
- BBR congestion control by default for QUIC, plus `--quic-cc bbr|cubic|newreno`
  for comparison and troubleshooting.
- QUIC throughput tuning: 64 MiB stream/connection windows, 64 MiB send window,
  8 MiB UDP socket buffers, PMTUD (1200–1452), low ACK delay, and parallel
  connection attempts across all IPv6 candidates.
- `--quic-mtu` for the initial QUIC UDP payload; `--wg-mtu` remains WireGuard-only.
- QUIC end-to-end tests (raw stream + HTTP/SHA-256) and an ignored 256 MiB
  throughput benchmark.

### Fixed

- QUIC now has the sender initiate the connection (matching WireGuard's firewall
  behavior), while the receiver acts as the QUIC server and sends UDP pinhole
  probes to open stateful-firewall return paths. The HTTP stream is still opened
  by the receiver and read by the sender.
- Peer cancellation is now transport-agnostic: QUIC uses explicit close code 1,
  maps stream/connection resets to a typed peer-cancelled marker, and the CLI
  prints `peer cancelled the transfer` without an `error:` prefix. Local Ctrl+C
  prints `interrupted`.

### Changed

- Transport HTTP/1.1 moved from `transport::wireguard::http` to the neutral
  `transport::http`, so QUIC does not depend on the WireGuard module.
- `orchestrate` is generic over `TransportConnection`; `session` wraps the opaque
  endpoint payload with a small transport header (inside AEAD) so the receiver can
  auto-select the implementation.
- The transport interface now exposes `set_credential_fingerprint`, a generic
  `AbortHandle`, and `shutdown`.

## [0.1.1]

### Added
- TCP/bridge/WireGuard statistics in the transport driver (debug every 2 s,
  info summary every 10 s): per-second tx/rx bytes, TCP send/receive queue
  occupancy, retransmissions, duplicate ACKs, bridge queue-full counts, and
  WireGuard delivery liveness.
- Android ARM64 build job in CI (NDK r29 / Android 16), released alongside
  Linux and Windows binaries.

### Fixed
- `hermod trust` now normalizes the URL (appends `:4376`) before fetching the
  SPKI, so pinning a server whose port was omitted no longer pins the wrong
  certificate.
- `hermod trust` sets the server as the default (`server_url`).
- Ctrl+C cancels the transfer immediately on both sides: the driver sends a
  TCP RST and the peer's bridge is closed, so the other side exits at once
  instead of waiting for the 30 s idle timeout.
- The peer now reports `peer cancelled the transfer` instead of a low-level
  I/O error.

## [0.1.0]

### Added

### Added
- Complete Rust implementation of the Hermod protocol:
  - CPace PAKE over P-256 (RFC 9496) with RFC 9380 hash-to-curve
  - HKDF-SHA256 key derivation, AES-256-GCM, EFF 1296-word transfer codes
  - WebSocket signaling client with SPKI certificate pinning
  - User-space WireGuard data plane (boringtun) over IPv6 UDP
  - In-process TCP/IP stack (smoltcp) with tunnel-internal HTTP/1.1
  - `hermod` CLI: `tx`, `rx`, `trust`, `serve` (serve deferred, D13)
- Structured, level-based logging via `tracing` (none/error/warning/info/debug)
- Level- and per-`--quiet`-aware status output aligned with the Go version
- Transfer integrity: both sides hash SHA-256 while streaming; the sender only
  completes after the receiver's digest matches (atomic temp-file rename)

### Fixed
- Path-MTU blackhole on real networks: default inner MTU 1400, `--wg-mtu`
  override, and CUBIC congestion control (smoltcp `socket-tcp-cubic`).
- WireGuard session expiry on long transfers: periodic rekey every 100 s.
- Receiver kernel UDP-buffer overflow: batch receive (up to 512 packets per
  smoltcp poll) and per-packet allocation elimination.
- Boringtun queued-packet handling: flush the internal queue after the
  handshake completes.
- `/ws` path handling and TOFU certificate fetch aligned with the Go server.

### Removed
- `--verify` / SAS out-of-band verification (design decision D12).
- IPv4, NAT hole punching, and UDP reflector (IPv6-only, D7).
