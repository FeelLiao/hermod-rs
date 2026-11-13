# hermod-core

`hermod-core` is the only home for Hermod's protocol logic: cryptography,
rendezvous (signaling), data-plane transports, the direct-stream payload protocol, session
negotiation and the final send/receive orchestration. Both front ends — the
`cli` binary (`hermod`) and the Windows GUI through `flutter-bridge` — are thin
shells that call into this crate; neither implements transfer state, trust,
hashing, inbox storage, timeouts or QUIC internals (decision D17 in the
original design docs).

This README explains how the crate is organized and why. Module-level `//!`
docs contain the line-by-line details; start with `src/lib.rs`.

## Mental model: one code, four layers

An entire transfer is a small protocol stack. Each layer only talks to its
neighbours:

```
┌────────────────────────────────────────────────────────────────┐
│ orchestrate   tx: serve_file/send_file/send_text               │  "the transfer"
│               rx: receive  → temp file → SHA-256 → atomic move │
├────────────────────────────────────────────────────────────────┤
│ transport     QUIC (TLS 1.3, pluggable CC, BBR default)         │  "a reliable,
│               direct-stream payload framing                     │   authenticated
│                                                                 │   byte stream"
├────────────────────────────────────────────────────────────────┤
│ session       transfer code → signaling → CPace → established  │  "getting the
│               connection + per-session credential              │   peers together"
├────────────────────────────────────────────────────────────────┤
│ crypto + signal + transfer/config/options utilities            │  "primitives"
└────────────────────────────────────────────────────────────────┘
```

Two properties to keep in mind when reading the code:

1. **The signaling server is a dumb relay.** It routes WebSocket messages by
   `channel_id` and never learns the transfer-code words, the shared secret,
   or the (cipher-protected) endpoint bundles. After the handshake the server
   is dropped entirely; payloads travel peer-to-peer over UDP.
2. **Every secret is derived by both peers locally** from the CPace shared
   key `K`. Nothing else enters the wire.

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
│   └── tls.rs        #   TLS client with SPKI pinning (fingerprint, not CA)
├── transport/
│   ├── iface.rs      # ★ the contract: Transport / TransportConnection traits
│   ├── mod.rs        #   AnyTransport/AnyConnection runtime dispatch
│   ├── network.rs    #   address enumeration / candidate filtering (shared)
│   ├── quic/         #   implementation: quinn + rustls, self-signed pins
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
│   ├── server_e2e.rs #   real TLS signaling service
│   └── e2e_full_chain.rs
└── examples/         # manual probes; ipv6_load_probe backs latency-check.sh
```

Naming conventions to keep in mind:

- **tx / Sender** = the side that serves the file (listens on the tunnel).
- **rx / Receiver** = the side that opens the payload stream (dials).
  `Transport::establish(Role, psk, peer_endpoint)` encodes this split, and the
  the payload framing respects it (`tx` listens, `rx` dials).
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

Key design decision: endpoint bytes are **opaque to the upper layers**. Each
implementation defines its own bundle format (QUIC: cert fingerprint + port
+ address candidates); the session layer
only encrypts, carries and unwraps them. Adding a transport means adding an
`AnyTransport`/`AnyConnection` arm, a `TransportKind` wire code, and per-protocol
validation in `options.rs` — `session.rs` and `orchestrate.rs` do not change.

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
JSON as an optional field, so old peers simply ignore it while new peers learn
the file size before the first payload byte.

### orchestrate.rs — "the transfer" (direct-stream framing, protocol v2)

One frame conversation on a single QUIC bidirectional stream (metadata comes

```
rx: writes GO                       tx: streams exactly N payload bytes
                                    hashing on the fly
rx: reads N bytes into .hermod_tmp (O_EXCL, 0600), hashes locally
rx: writes verdict [status|sha256|size]  → tx accepts or rejects (verified)
rx: rename temp → safe_destination_path (dir-stripped, de-duplicated)
rx: rename temp → safe_destination_path (dir-stripped, de-duplicated)
```

`orchestrate::receive` is the safety boundary: nothing touches the final path
unless the digest verdict passed. Cancellation travels as a dedicated io error
marker up through `OrchestrateError::PeerCancelled`, so hosts can print the
friendly "peer cancelled the transfer" without string matching.

## Runtime, errors, logging — conventions for hosts

- **Runtime ownership**: `hermod-core` never builds a tokio runtime and never
  blocks. Hosts (CLI, GUI bridge worker) own the runtime and drive the futures.
- **Cancellation**: everything cancellable hangs off `AbortHandle` (WG sends an
  RST; QUIC uses a CONNECTION_CLOSE code). Hosts keep one slot and call
  `abort()` on Ctrl+C / the GUI cancel button.
- **Logging**: `log_info!`/`log_warn!`/… are `tracing` macros. The crate does
  not install a subscriber; CLI ties `--verbose` to a filter, the GUI logs into
  a bounded in-memory ring.
- **Errors** are typed per layer (`SessionError`, `OrchestrateError`,
  `TransportError`, `BundleError`, `CpaceError`, …). Never `unwrap()` on peer
  input: `transfer::validate_filename` + path-safety guards exist because
  hostile names/paths arrive from the other end.
- **Workspace safety**: `unsafe_code = "forbid"` applies workspace-wide; this
  crate contains no unsafe.

## Configuration and trust

`config.rs` keeps the Go-compatible YAML schema (`server_url`,
`trusted_servers: {url: spki_fingerprint}` and the (unused) TLS preference
fields, so old files still load). `normalize_server_url` pins the default port
and `expected_spki` refuses to run against a server that is not pinned — a
transfer never starts without verified signaling-server identity.

`options.rs` is the *shared* surface for CLI flags and GUI settings:
`ConnectionSettings::session_options(receiving)` validates every field against
the chosen transport (MTU ranges, congestion support, receiver `auto`) BEFORE
any network access; both front ends call exactly this code path.

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
  test and `latency-check.sh`.

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
points are unchanged. See the [CLI device guide](../cli/README.md) for commands,
file locations, platform permission requirements and recovery rules.

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

The older root design draft describes the
motivation but its automatic-pairing, Go-server and key-size details are superseded
by this implemented CLI/core contract; no GUI or Go code is changed.

## See also

- `../README.md` / `../README.zh-CN.md` — user-facing setup and migration.
- `../AGENTS.md` — repo conventions and the CLI/GUI boundaries.
- `../APP_BUILD.md` — Flutter toolchain, Store identity and CI gates.
- `../NETWORK_PERFORMANCE.md` — why QUIC/BBR exists and its tuning knobs.
