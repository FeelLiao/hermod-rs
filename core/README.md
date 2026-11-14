# hermod-core

`hermod-core` owns the shared cryptography, signaling, device identity, pairing,
QUIC transport and file-transfer orchestration. The CLI parses commands and presents
results; application-specific settings, jobs and inbox policy are outside this crate.
Start with [crate documentation](src/lib.rs) and the modules below.

## Mental model

```text
CLI command or embedding application
  → session: code rendezvous or authenticated device rendezvous
  → transport: QUIC connection and bidirectional streams
  → orchestrate: payload v3, exact byte counts, hashing and receiver verdict
```

`crypto`, `identity`, `contacts`, `config` and `options` supply the primitives and
validated state. `server` implements the optional Rust signaling service.

- The server routes encrypted endpoint messages without learning code passwords,
  session keys, pairing secrets or file contents. Device registration separately
  proves private-key possession; it does not establish peer trust. The server can
  observe device IDs, call relationships, source addresses and timing.
- Per-session credentials derive from CPace's shared key. Long-term device private
  keys are generated and stored separately. They are not derived from a transfer
  code. Pairing recovery can transmit a saved secret inside an authenticated,
  encrypted pairing exchange; it is never exposed to the signaling service.

## Directory map

```
core/src/
├── lib.rs            # crate docs, re-exports, the only entry users need
├── crypto/           # primitives ONLY; no network code here
│   ├── cpace.rs      #   CPace PAKE over P-256 (RFC 9380/9496), Go-compatible
│   ├── kdf.rs        #   HKDF: K → channel-bound PSK + session token
│   ├── aead.rs       #   AES-256-GCM for endpoint bundles (nonce‖ct‖tag)
│   └── wordlist.rs   #   EFF short wordlist; transfer code gen/parse
├── signal/           # rendezvous layer (WebSocket to the signaling server)
│   ├── protocol.rs   #   wire format (allocate/join/blob/ok/ready/error)
│   ├── client.rs     #   connect + CPace handshake orchestration
│   ├── device.rs     #   authenticated /device control protocol
│   └── tls.rs        #   TLS client with SPKI pinning (fingerprint, not CA)
├── transport/
│   ├── iface.rs      # ★ the contract: Transport / TransportConnection traits
│   ├── mod.rs        #   AnyTransport/AnyConnection runtime dispatch
│   ├── network.rs    #   address enumeration / candidate filtering (shared)
│   ├── quic/         #   implementation: quinn + rustls, self-signed pins
├── server/           # TLS service, device routing and generation-owned channels
├── identity.rs       # persistent P-256 identity and canonical device ID
├── identity/storage.rs # private files, permissions, transaction locks
├── contacts.rs       # remarks, pending/active pairings, sending lease
├── session.rs        # one hop: transfer code → established connection
├── session/          # signed pairing and dual-door device rendezvous
├── orchestrate.rs    # tx/rx framing + verdict on top of a connection
├── transfer.rs       # payload semantics: Kind, paths, hashing, offers
├── config.rs         # Go-compatible YAML schema, trusted-server pins
├── options.rs        # shared CLI/GUI connection settings → SessionOptions
└── log.rs            # tracing macros; the HOST installs the subscriber
```

Extras that Cargo knows about:

```
core/
├── tests/            # public-API integration targets
│   ├── common/       #   one-shot local signaling relay + test Config helpers
│   ├── orchestrate.rs #  orchestration integration entry point
│   ├── orchestrate/  #   memory and QUIC integration cases
│   ├── quic_transport.rs # endpoint lifecycle and authentication
│   ├── device_*.rs   #   identity, storage, pairing, dual-door and ID transfer
│   ├── server_e2e.rs #   real TLS signaling service
│   └── e2e_full_chain.rs
└── examples/         # manual probes; ipv6_load_probe exercises the current QUIC stream
```

Naming conventions to keep in mind:

- **tx / Sender** = the side that serves the file (listens on the tunnel).
- **rx / Receiver** = the side that opens the payload stream (dials).
  `Transport::establish(Role, psk, peer_endpoint)` encodes this split, and the
  payload framing respects it (`tx` listens, `rx` dials).
- The crate *package* is `hermod-core`; the workspace *directory* is `core`.

## Layer contracts

### transport/iface.rs — the one seam that matters

Everything in `session`/`orchestrate` is written against two traits:

```rust
trait Transport {                       // lifecycle per transfer
    fn set_credential_fingerprint(&mut self, [u8;32]);  // SHA-256(once-token)
    async fn prepare(&mut self) -> Result<Vec<u8>>;     // opaque endpoint bytes
    async fn establish(role, psk, peer_endpoint) -> Self::Connection;
}

trait TransportConnection {             // established, ready to carry bytes
    async fn listen(&mut self) -> Result<Self::Stream>;   // tx side
    async fn dial(&mut self)   -> Result<Self::Stream>;   // rx side
    fn abort_handle(&self) -> AbortHandle;  // protocol-agnostic cancel
    async fn shutdown(&mut self);           // flush the final verdict frames
    fn network_stats(&self) -> Option<NetworkStatsHandle>; // QUIC only today
}
```

Transport-specific endpoint bytes are interpreted by the selected transport.
The session layer wraps them, attaches transfer metadata and capability information,
and encrypts/authenticates the exchange. Device sessions additionally bind endpoint
bytes into a signed transcript. Keep these responsibilities separate when extending
an implementation; adding a transport is not guaranteed to require zero session changes.

### session.rs — from words to wire

`sender_session_with_ready` / `receiver_session_with` take the *transfer code*
(e.g. `4660-acid-acorn-acre` = channel id + PAKE password words) and produce a
connected `TxSession`/`RxSession`:

1. connect to the signaling server (SPKI pinned, 15 s timeout);
2. `allocate` / `join`; the `ok` payload carries the server-observed public
   address (adopted as an IPv6 hint only);
3. CPace handshake → shared `K` (wrong code = different `g` = dead session);
4. derive locally, never over the wire:
   `token  = HKDF(K, cid, "hermod-http-token-v1")  → per-session credential fingerprint`,
   `psk = HKDF(K, cid, "hermod-wg-psk-v1")`,
   `fingerprint = SHA-256(token)`;
5. exchange AEAD-encrypted endpoint bundles (`blob3`/`blob4`), each wrapped
   with a small `HE`-header whose 4th byte tells the receiver which transport
   to build — that is how an auto receiver follows the sender;
6. `establish` and return. The signaling WebSocket is dropped here.

The receiver-side `transfer_offer` (kind/name/size) piggybacks on the endpoint
JSON field. The current receiver requires this offer to obtain the byte count
before payload transfer; incompatible endpoint protocol versions are rejected.
The offer contains kind, name and size, not a precomputed SHA-256.

### orchestrate.rs — payload protocol v3

Metadata arrives in the encrypted signaling exchange before QUIC payload transfer.
The receiver opens a control stream and selects the mode:

- **Single stream**: `MODE_SINGLE` (1 byte), followed by exactly the announced
  payload size from tx to rx. Both sides calculate SHA-256 while processing bytes.
- **Multiple streams**: `MODE_MULTI` plus a 12-byte plan (little-endian u64 slice
  size and u32 slice count). The receiver opens bounded data streams carrying
  slice indices; the sender seeks the source file, while the receiver writes each
  slice at its offset and hashes the complete contents in file order.
- **Receiver verdict**: rx sends `[status:1 | sha256:32 | size:8 LE]` back to tx,
  on the payload stream or the multi-stream control stream. SHA-256 is binary on
  the wire, not hex text. Tx compares all fields with its local result.

The receiver writes a temporary file, computes its own digest, sends the verdict,
synchronizes the file and renames it to a sanitized, non-clashing destination.
**There is currently no tx-to-rx final digest-acceptance frame.** The receiver does
not wait for sender agreement before finalizing its file. QUIC provides transport
integrity, but this is not a symmetric application-level digest-confirmation protocol.
Device-pairing confirmation does not change this payload behavior.

Cancellation and error paths discard incomplete temporary files. Transport errors
carry a peer-cancellation marker, exposed as `OrchestrateError::PeerCancelled`.

## Runtime, errors, logging — conventions for hosts

- **Runtime ownership**: the embedding application owns the Tokio runtime.
  Identity/contact APIs perform synchronous filesystem operations, OS locking and,
  on Windows, permission-check subprocesses. Async callers must move these operations
  to `spawn_blocking`; session authentication already does this for contact transactions.
- **Cancellation**: QUIC transfers expose `AbortHandle` for immediate connection
  cancellation. Dropping rendezvous futures closes their owned signaling connections;
  candidate and confirmation stages have bounded deadlines.
- **Logging**: `log_info!`/`log_warn!`/… are `tracing` macros. The crate does
  not install a subscriber; CLI ties `--verbose` to a filter, the GUI logs into
  a bounded in-memory ring.
- **Errors** are typed per layer (`SessionError`, `OrchestrateError`,
  `TransportError`, `BundleError`, `CpaceError`, …). Never `unwrap()` on peer
  input: `transfer::validate_filename` + path-safety guards exist because
  hostile names/paths arrive from the other end.
- **Safety**: core and CLI forbid handwritten unsafe Rust; the separately generated
  FFI bridge glue has its own exception. This crate contains no unsafe code.

## Configuration and trust

`config.rs` keeps the Go-compatible YAML schema (`server_url`,
`trusted_servers: {url: spki_fingerprint}` and the (unused) TLS preference
fields, so old files still load). `normalize_server_url` pins the default port
and `expected_spki` refuses to run against a server that is not pinned — a
transfer never starts without verified signaling-server identity.

`options.rs` provides shared connection settings and runtime tuning validation.
The CLI builds `SessionOptions` directly and validates them before networking;
other callers can use `ConnectionSettings::session_options(receiving)`. These
are not identical parsing paths. Runtime tuning is not read from or saved to YAML.
The current configuration retains only the selected signaling server pin. Device
contacts are a separate trust domain and may contain multiple paired devices.

## Testing model

- **Crypto**: independent RFC vectors (hash-to-curve, CPace vectors extracted
  from the Go suite) — byte-compatibility with the Go implementation is the
  acceptance criterion, see `crypto/` tests.
- **Wire formats**: `signal::protocol` round-trips against JSON references.
- **Transports**: handshake/frame/streams/digest/cancel tests run QUIC
  implementations against local loops; full candidate paths additionally
  require a private IPv4 or IPv6 test interface (see root CI).
- **Public-API integration**: [orchestration tests](tests/orchestrate.rs) drive
  memory streams and real QUIC pairs, including distinct chunk sizes and bounded
  concurrency. [Full-chain tests](tests/e2e_full_chain.rs) add session negotiation
  and cancellation over an in-process relay; [server tests](tests/server_e2e.rs)
  exercise the real TLS signaling service.
- **Process integration**: [CLI interoperability](../cli/tests/interop.rs) runs
  the actual binary against the GUI engine; [CLI arguments](../cli/tests/arguments.rs)
  checks standard streams, usage errors and exit codes.
- **Performance**: correctness gates (fmt/test/clippy/doc/deny) do not establish
  throughput improvement. Manual probes include the ignored `quic_throughput`
  test and the `ipv6_load_probe` example. These are not release gates.

### Source and test placement

Rust distinguishes tests by visibility and compilation boundary, not by whether
networking or async code is involved:

- Small unit tests may remain inline as `#[cfg(test)] mod tests { ... }`.
- Larger private unit suites use an ordinary child module: `#[cfg(test)] mod tests;`
  in the implementation and `src/<module>/tests.rs` for the test body. They can
  inspect private helpers without exposing them as public APIs. This is a layout
  convention, not a requirement to create a separate test file for every module.
- Tests that exercise only public APIs belong in the crate-root `tests/`
  directory. Each top-level Rust file is a separate Cargo integration target.
  Related cases live in submodules such as `tests/orchestrate/{memory,quic}.rs`
  under the single `tests/orchestrate.rs` entry point.
- Shared integration helpers belong in `tests/common/mod.rs` (or a similarly
  named subdirectory), not a top-level helper file that Cargo treats as a test target.
- Never add `pub`, `include!` or a cross-directory `#[path]` just to let an
  integration test access private production code. Private assertions stay unit tests.

Both `name.rs` + `name/` and `name/mod.rs` are valid Rust module layouts;
`mod.rs` is not deprecated. Follow the surrounding module's convention rather
than mechanically renaming unrelated production files.

## Embedding

See the example in `src/lib.rs`: two `QuicTransport`s can be established
end-to-end without any signaling server, given a `K` and a channel id — useful
for tests and for any future app that already shares a secret out-of-band.

## Device pairing and ID rendezvous

The CLI now opts into long-term pairing on both sides with `--pair`. Ordinary
code transfers keep their existing protocol and do not save contacts. New public
session entry points are `sender_device_session`, `receiver_pairing_session`,
`sender_dual_session` and `receiver_device_session`; GUI callers of existing entry
points are unchanged. See the [English CLI guide](../README.md#cli-device-pairing-and-remarks)
or [Chinese CLI guide](../README.zh-CN.md#cli-设备配对与备注) for commands, file locations,
platform permission requirements and recovery rules.

A sender owns its normal `/ws` code connection and an independent authenticated
`/device` control connection. Each ID candidate uses a separate channel; CPace
uses the saved secret, while signatures pin the expected device and bind the
ordered endpoint transcript. File direction is distinct from channel creation
and CPace send order. Only an authenticated winner proceeds to confirmation;
the other entry is cancelled. Neither call notification nor registration proof
establishes end-to-end peer trust.

Identity and contacts live under `config_dir/device-state/`. Storage fails closed
on invalid schema or permissions; writes use cross-process locks and atomic
replacement. Pairing confirmation supports matching pending/active recovery,
but is not a distributed atomic transaction. Existing active secrets are not
silently replaced, and file-transfer success remains a separate outcome.

The server's registry and pending calls are in memory, bounded and associated
with connection IDs. Registration proves possession of the claimed identity;
channel-owner tokens restrict calls to an allocated candidate channel. No
account database, contact list or pairing secret is stored on the server.
QUIC payload v3 remains unchanged. New acceptance suites are
`device_storage`, `device_control`, `device_pairing`, `device_dual` and
`device_sessions`; they cover private storage, registration proof, interrupted
confirmation, forged candidates and real-server file transfer. Windows tests
must run under a token allowed to set and verify the test files' private ACLs;
restricted execution is expected to fail closed, not skip the check. Linux
storage permission tests are platform-specific and require a working native C
linker in addition to Rust.

The implemented contract uses explicit bilateral pairing, a 33-byte canonical
P-256 compressed public key and the Rust server control endpoint. It does not
require a Go server extension or introduce a GUI device-management interface.

## See also

- [English](../README.md) / [简体中文](../README.zh-CN.md) — user-facing setup and migration.
- [Build and release guide](../APP_BUILD.md) — toolchains, packaging and CI gates.
- [CLI tuning and resource tradeoffs](../README.md#cli-transport-options).
