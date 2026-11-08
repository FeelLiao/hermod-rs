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
├── session.rs        # one hop: transfer code → established connection
├── orchestrate.rs    # tx/rx framing + verdict on top of a connection
├── transfer.rs       # payload semantics: Kind, paths, hashing, offers
├── config.rs         # Go-compatible YAML schema, trusted-server pins
├── options.rs        # shared CLI/GUI connection settings → SessionOptions
└── log.rs            # tracing macros; the HOST installs the subscriber

Extras that Cargo knows about:

```
core/
├── tests/            # public-API acceptance layer, driven by an in-process relay
│   ├── common/       #   one-shot local signaling relay + test Config helpers
│   └── e2e_full_chain.rs
└── examples/         # manual probes; ipv6_load_probe backs latency-check.sh
```
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
- **End-to-end**: `orchestrate` tests drive WG+QUIC pairs through real SDU
  pipelines; `cli/tests/interop.rs` runs the actual binary against the GUI
  engine over a local relay, and `core/tests/e2e_full_chain.rs` is the
  acceptance layer that opens the crate's public API the way external hosts
  do — full session + transfer + cancel over an in-process relay.
  `cargo bench` benchmarks were deliberately not adopted: correctness gates
  (fmt/test/clippy/doc/deny) are the automated bars, while performance work
  uses the manual probes (`quic_throughput` ignored test, `latency-check.sh`).

Folder layout follows standard Rust 2018: `src/lib.rs` plus `mod` trees; a
module with sub-files either uses `name.rs` + `name/` directory (XML-era
style, as in `transport.rs` + `transport/quic.rs`) or `name/mod.rs`
(deeper modules like `transport/quic`). Keep it that way — nesting mirrors the
module hierarchy, and `cargo` tooling (doc, fmt, clippy) all assume it.

## Embedding

See the example in `src/lib.rs`: two `QuicTransport`s can be established
end-to-end without any signaling server, given a `K` and a channel id — useful
for tests and for any future app that already shares a secret out-of-band.

## Where the protocol is heading

`../docs/device-id-pairing.md` drafts the planned device-identity/pairing
extension (long-term keys, contacts, "call by device id" alongside the
transfer-code flow). It reuses every layer described here; only
`signal/` and `session.rs` gain new entry points.

## See also

- `../README.md` / `../README.zh-CN.md` — user-facing setup and migration.
- `../AGENTS.md` — repo conventions and the CLI/GUI boundaries.
- `../APP_BUILD.md` — Flutter toolchain, Store identity and CI gates.
- `../NETWORK_PERFORMANCE.md` — why QUIC/BBR exists and its tuning knobs.
