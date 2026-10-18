# Agent Guide

Guidance for LLM agents working on this repository.

## Read first

- `README.md` — what the project is and how it is used
- `CHANGELOG.md` — recent changes
- The source tree in `hermod-core/` — the module `//!` docs explain each layer
  (crypto, signal, transport, session, orchestrate)

## Conventions

- **Language:** code identifiers, log messages, and user-facing output are in
  English; code comments are in Chinese (zh-CN).
- **Commits:** Conventional Commits (`<type>(<scope>): <summary>`).
- **Quality gate (must pass before merging):**
  ```bash
  cargo test --workspace
  cargo clippy --workspace --all-targets -- -D warnings
  ```
  `unsafe` code is forbidden (workspace `lints.rust.unsafe_code = "forbid"`).

## Architecture (one paragraph)

`hermod-core` holds everything: crypto primitives (`crypto/`), the WebSocket
signaling client with SPKI pinning (`signal/`), the `Transport` trait and its
WireGuard implementation (`transport/` — boringtun tunnel + smoltcp in-process
TCP + in-tunnel HTTP), the session orchestration that turns a transfer code into
an established transport (`session.rs`), and the tx/rx main flow
(`orchestrate.rs`). `hermod-cli` is a thin clap shell over `hermod-core`.

## Testing notes

- The crypto modules use official vectors (RFC 7748, RFC 9380, RFC 5869) as
  independent baselines.
- The transport has local end-to-end tests that exercise the full chain
  (WireGuard handshake → smoltcp TCP → HTTP → SHA-256 verdict) over the host's
  own IPv6 addresses, so no external server is needed.
- Tests that need a real signaling server are not run by CI in this sandbox;
  verify against a live server before relying on them.
