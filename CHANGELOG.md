# Changelog

All notable changes to this project are documented in this file. The format is
based on [Keep a Changelog](https://keepachangelog.com/), and this project
adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Optional **QUIC data plane** (`--transport quic`) as a standalone transport
  implementation; WireGuard remains the default and is unchanged.
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

### Changed

- Transport HTTP/1.1 moved from `transport::wireguard::http` to the neutral
  `transport::http`, so QUIC does not depend on the WireGuard module.
- `orchestrate` is generic over `TransportConnection`; `session` treats endpoint
  bytes as opaque and selects the transport at runtime.
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
