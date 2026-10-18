# Hermod (Rust)

Hermod transfers files and text directly between peers over an encrypted WireGuard
tunnel. No data passes through the signaling server — it only helps the two peers
find each other.

The project is named after Hermod, the messenger god in Norse mythology — the primary
courier of the Æsir, best known for riding to Hel to negotiate the return of Baldr.

Hermod was inspired by [Magic Wormhole](https://github.com/magic-wormhole/magic-wormhole),
a Python tool that transfers files between computers using short, human-readable codes.

This is a **from-scratch Rust implementation** of the Hermod protocol. The Go
implementation lives in the sibling `hermod-go` repository and shares the same
signaling protocol, CPace PAKE, and transfer-code format.

## What it does

- End-to-end encrypted transfer over **user-space WireGuard** (boringtun) carrying a
  small in-process TCP/IP stack (smoltcp) with tunnel-internal HTTP/1.1
- Password-authenticated key exchange (**CPace over P-256**) — no shared secret beyond
  the transfer code
- **Zero privilege**: everything runs in userspace — no root, no TUN device, no network
  interface changes
- **IPv6-only** (no IPv4, no NAT hole punching)
- Transfers files, text snippets, or stdin
- Works with the Go signaling server (or any hosted one)

## Quick start

**Build:**
```bash
cargo build --release -p hermod-cli
# → target/release/hermod
```

**Fully static binary (musl)** — no glibc dependency:
```bash
rustup target add x86_64-unknown-linux-musl
sudo apt install musl-tools cmake        # cmake is needed by aws-lc-sys (rustls)
cargo build --release --target x86_64-unknown-linux-musl -p hermod-cli
# → target/x86_64-unknown-linux-musl/release/hermod
```

**Start a signaling server** (one machine, reachable by both peers) — the Rust version
does not ship a server yet (D13); use the Go server:
```bash
# from the hermod-go repository
./hermod serve
```

**Pin the server certificate** (run once on each client):
```bash
./hermod trust wss://your-server:4376
```

**Send a file:**
```bash
./hermod tx report.pdf
# Prints a transfer code to stderr, e.g.: 3-apple-banana-cherry
```

**Receive:**
```bash
./hermod rx 3-apple-banana-cherry
```

The file lands in the current directory.

> **IPv6 requirement:** both peers need IPv6 connectivity. The WireGuard tunnel is
> established over IPv6 UDP; if a peer is behind a firewall that blocks inbound UDP,
> the connection may fail. There is no IPv4 fallback or NAT traversal.

## Usage

### Global flags

| Flag | Short | Default | Description |
|---|---|---|---|
| `--verbose` | | `none` | Log verbosity: `none`, `error`, `warning`, `info`, `debug` |
| `--quiet` | `-q` | off | Suppress status output. Errors are always shown. |
| `--version` | `-V` | | Print the version and exit. |

### tx — send

```
hermod tx [INPUT] [flags]
```

`INPUT` can be:
- A file path — sends the file
- A quoted string — sends it as text
- `-` — reads from stdin

| Flag | Short | Default | Description |
|---|---|---|---|
| `--server` | `-s` | `wss://oracle.stariverfeel.eu.org:4376` | Signaling server URL (saved as default when specified) |
| `--words` | `-w` | `3` | Number of words in the transfer code |
| `--listen` | `-l` | `[::]:0` | Local UDP bind address (IPv6) |
| `--wg-mtu` | | `1400` | Inner IP MTU (outer packet = inner + 80) |

**Examples:**
```bash
# Send a file
./hermod tx photo.jpg -s wss://relay.example.com:4376

# Send text
./hermod tx "Meeting at 3pm"

# Send stdin (pipe)
tar czf - ./project | ./hermod tx -

# Smaller inner MTU for lossy/PPPoE paths (outer ≈ 1280 bytes)
./hermod tx big.fa --wg-mtu 1200
```

### rx — receive

```
hermod rx [CODE] [flags]
```

| Flag | Short | Default | Description |
|---|---|---|---|
| `--server` | `-s` | `wss://oracle.stariverfeel.eu.org:4376` | Signaling server URL (saved as default when specified) |
| `--destination` | `-d` | current directory | Output directory |
| `--listen` | `-l` | `[::]:0` | Local UDP bind address (IPv6) |
| `--wg-mtu` | | `1400` | Inner IP MTU (must match the sender) |

**Examples:**
```bash
# Receive to current directory
./hermod rx 3-apple-banana-cherry -s wss://relay.example.com:4376

# Save to a specific directory
./hermod rx 3-apple-banana-cherry -d ~/downloads/
```

### serve — run a signaling server

Not implemented in the Rust version (D13). Use the Go server from the sibling
`hermod-go` repository. `hermod serve` prints an error and exits non-zero.

### trust — pin a server certificate

```
hermod trust [SERVER_URL] [--fingerprint FINGERPRINT]
```

Connects to the server, fetches its certificate SPKI fingerprint, saves it to the local
config, and sets the server as the default for future `tx` and `rx` calls. If you omit
the port, `trust` defaults to port 4376.

| Flag | Short | Default | Description |
|---|---|---|---|
| `--fingerprint` | | `""` | Expected SHA-256 fingerprint (hex). When set, the server's TLS certificate is verified against this value during the handshake. |

**Security note:** The initial connection uses no certificate verification
(trust-on-first-use). Run `hermod trust` over a trusted network — for example, a VPN,
physical LAN, or a network where the fingerprint can be confirmed out-of-band. If you
already know the server's fingerprint, pass it with `--fingerprint` to verify before
pinning.

## Configuration

Hermod stores its config in:
- Linux/macOS: `~/.config/hermod/config.yaml`
- Windows: `%APPDATA%\Hermod\config.yaml`

The config holds the `server_url`, TLS settings, and the `trusted_servers` map
(normalized server URL → SPKI fingerprint).

## How it works

1. The sender connects to the signaling server and allocates a channel, receiving a
   numeric channel ID.
2. The transfer code encodes the channel ID plus a random word passphrase.
3. The receiver connects to the signaling server using the code and joins the channel.
4. Both peers enumerate their local IPv6 addresses; the server-observed address is used
   as a hint. No UDP reflector is needed — IPv6 has no NAT.
5. Both peers run a **CPace PAKE** handshake over the signaling channel to establish a
   shared key `K`, authenticated by the passphrase.
6. Each side derives a WireGuard pre-shared key `PSK = HKDF-SHA256(K, channel_id,
   "hermod-wg-psk-v1")` and generates an ephemeral X25519 key pair. An encrypted
   endpoint bundle (WireGuard public key + UDP port + IPv6 candidates) is AEAD-sealed
   with `K` and exchanged through the signaling relay — the server never learns it.
7. A **WireGuard handshake** is performed over IPv6 UDP. Both sides send handshake
   initiations to all candidates (opening firewall pinholes); the sender initiates, the
   receiver responds.
8. Inside the tunnel, an in-process TCP/IP stack (smoltcp) carries **HTTP/1.1**: the
   sender serves `GET /f/<token>`, the receiver pulls the whole payload while hashing
   SHA-256, then posts the digest to `POST /done`. The sender compares the digest with
   its own and only then completes the transfer.

## Tuning

- **LAN throughput**: the inner TCP window defaults to 1 MiB. On a fast LAN you can
  raise it with `HERMOD_TCP_SOCKET_BUF` (bytes, e.g. `4194304` for 4 MiB) on both
  sides. Keep it small on lossy WAN paths — a large window amplifies bufferbloat.
- Check `--verbose info`'s `receive progress ... write_pct`: if it is high, the
  receiving disk is the bottleneck, not the network.

## Security properties

| Property | Mechanism |
|---|---|
| Confidentiality | WireGuard with per-session ephemeral X25519 keys + a PSK derived from the CPace key |
| Authentication | CPace PAKE — only someone with the transfer code can connect |
| Integrity | SHA-256 verified on both sides; the sender renames/releases only after the digest matches |
| Server cannot read data | Payload never touches the signaling server; the endpoint bundle is encrypted with the CPace key |
| Session isolation | Fresh ephemeral keys and PSK per session; the PSK is never transmitted |

> Note: unlike the Go version, the Rust implementation is **classical-only** (no
> post-quantum ML-KEM) and **does not implement SAS out-of-band verification**
> (`--verify` was removed by design).

## Development

```bash
# Run all tests
cargo test --workspace

# Lint (warnings are errors)
cargo clippy --workspace --all-targets -- -D warnings
```

## Layout

```
hermod-core/           all protocol, tunnel, and transfer logic
  src/crypto/          CPace PAKE, HKDF, AES-256-GCM, EFF wordlist
  src/signal/          WebSocket signaling protocol, SPKI pinning, client
  src/session.rs       transfer code → established transport orchestration
  src/transport/       Transport trait + WireGuard implementation
    wireguard/wg/      X25519 keys, boringtun wrapper, endpoint roaming
    wireguard/stack/   smoltcp device/driver + AsyncRead/AsyncWrite bridge
    wireguard/http/    in-tunnel HTTP/1.1 (server + client)
  src/orchestrate.rs   tx/rx transfer main flow
hermod-cli/            thin CLI shell (clap) over hermod-core
```

## License

MIT — see [LICENSE](LICENSE).
