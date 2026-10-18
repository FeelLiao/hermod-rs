# Changelog

All notable changes to this project are documented in this file. The format is
based on [Keep a Changelog](https://keepachangelog.com/), and this project
adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

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
