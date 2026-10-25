# Hermod (Rust)

Hermod transfers files and text directly between peers over an encrypted tunnel.
**QUIC** (TLS 1.3 + BBR) is the default data plane; WireGuard remains available
with `--transport wireguard`. No data passes through the signaling server — it
only helps the two peers find each other.

The project is named after Hermod, the messenger god in Norse mythology — the primary
courier of the Æsir, best known for riding to Hel to negotiate the return of Baldr.

Hermod was inspired by [Magic Wormhole](https://github.com/magic-wormhole/magic-wormhole),
a Python tool that transfers files between computers using short, human-readable codes.

This is a **from-scratch Rust implementation** of the Hermod protocol. The Go
implementation lives in the sibling `hermod-go` repository and shares the same
signaling protocol, CPace PAKE, and transfer-code format.

## What it does

- End-to-end encrypted transfer over **QUIC** (quinn) with TLS 1.3 certificate
  pinning and BBR congestion control, or over **user-space WireGuard** (boringtun)
  with a small in-process TCP/IP stack (smoltcp)
- The sender chooses the transport; the receiver defaults to **auto** and follows
  the sender's endpoint header. `--transport` can still force a value on either side.
- Password-authenticated key exchange (**CPace over P-256**) — no shared secret beyond
  the transfer code
- **Zero privilege**: everything runs in userspace — no root, no TUN device, no network
  interface changes
- **IPv6-only** (no IPv4, no NAT hole punching)
- Transfers files, text snippets, or stdin
- Works with the Go signaling server (or any hosted one)

## Quick start

**Tauri app (Windows, Linux, macOS and Android):**
```bash
cd hermod-app
npm ci
npm run tauri dev
# Package on the current desktop OS:
npm run tauri build
```

The GUI uses **Tauri 2 + Material Design 3**, with a desktop navigation rail,
phone bottom navigation, system/light/dark themes, sending/receiving, code copying,
cancellation, progress, RTT, SHA-256 verification and an editable bandwidth budget.
The old egui/eframe app has been replaced. Node.js is needed to build, not to run
an installed app. Fonts and UI assets are bundled; no CDN is contacted.

Use the system picker to select a file. Completed downloads appear in the private
**Inbox**; **Save a copy** opens the platform's save dialog. Android uses authorized
`content://` document streams, not guessed filesystem paths or broad storage
permissions. Export important files before uninstalling the app.

The GUI starts with an **8 Mbps payload cap**. Adjust this to about 70–85% of your
actual upload capacity; 8 Mbps is only a starting value, not a bandwidth estimate.
CLI defaults remain unlimited. Both peers still need reachable **IPv6**.

Linux build dependencies (Ubuntu 22.04+):
```bash
sudo apt install build-essential pkg-config cmake perl clang \
  libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev patchelf
```

See [APP_BUILD.md](APP_BUILD.md) for Android setup, file lifecycle, installation,
release artifacts and platform testing. Desktop builds reuse the CLI trust config
unless an app-specific `config.yaml` exists; Android uses the pinned default relay.

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

> **IPv6 requirement:** both peers need IPv6 connectivity. Both data planes are
> established over IPv6 UDP; if a peer is behind a firewall that blocks inbound UDP,
> the connection may fail. There is no IPv4 fallback or NAT traversal.

## Keep browsing responsive

An unrestricted upload can fill the uplink queue and delay other requests.
Both CLI commands accept `--limit-mbps` (decimal Mbps, 0 = unlimited):

```bash
# Example: on a measured 20 Mbps uplink, start with a 15 Mbps payload cap
./hermod tx report.pdf --limit-mbps 15
./hermod rx CODE --limit-mbps 15
```

The sender applies the lower of its own cap and the receiver's requested cap.
Use upgraded peers for sender-side pacing requested by the receiver. The GUI
starts at a visible 10 Mbps cap that you can change or disable. The CLI preserves
unlimited operation unless a cap is supplied. This is a payload budget, so allow
headroom for protocol overhead, retransmissions and other applications. Start
at 70–85% of measured **upload** capacity. See the
[investigation and reproducible latency comparison](NETWORK_PERFORMANCE.md).

## IPv6 in WSL 2

Adding a made-up IPv6 address does not provide an Internet route. If Windows
already has working IPv6 but the WSL NAT interface has only `fe80::` addresses,
use [Microsoft's mirrored networking mode](https://learn.microsoft.com/en-us/windows/wsl/networking#mirrored-mode-networking)
on supported Windows 11 versions. Back up `%USERPROFILE%\.wslconfig`, then add
this key to its existing `[wsl2]` section:

```ini
[wsl2]
networkingMode=mirrored
```

Save work in all WSL terminals and stop containers that must not be interrupted.
Run `wsl --shutdown` from **Windows PowerShell**, then reopen WSL and check:

```bash
ip -6 address show scope global
ip -6 route show default
```

The setting takes effect only after restarting WSL. A global address and default
route are prerequisites, not proof that the peer's UDP path is reachable. Host
and router firewalls must permit the transfer. Hermod does not disable firewalls
or assign duplicate Windows addresses to the guest.

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
| `--limit-mbps` | | `0` | Payload cap in Mbps; 0 = unlimited. Leave upload headroom for browsing. |
| `--transport` | | `quic` | Data plane: `quic` or `wireguard`; receiver auto-follows by default |
| `--wg-mtu` | | `1400` | WireGuard inner IP MTU (outer packet = inner + 80; WireGuard only) |
| `--quic-cc` | | `bbr` | QUIC congestion control: `bbr`, `cubic`, or `newreno` (QUIC only) |
| `--quic-mtu` | | `1200` | Initial QUIC UDP payload, clamped to 1200–1452; PMTUD may raise it (QUIC only) |

**Examples:**
```bash
# Send a file
./hermod tx photo.jpg -s wss://relay.example.com:4376

# Send text
./hermod tx "Meeting at 3pm"

# Send stdin (pipe)
tar czf - ./project | ./hermod tx -

# Default data plane is QUIC with BBR
./hermod tx big.fa

# Fall back to WireGuard (receiver auto-follows)
./hermod tx big.fa --transport wireguard --wg-mtu 1200

# QUIC congestion-control comparison; receiver auto-detects QUIC
./hermod tx big.fa --quic-cc cubic
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
| `--limit-mbps` | | `0` | Receiver payload cap in Mbps; sent to upgraded senders. 0 = unlimited. |
| `--transport` | | `auto` | `auto` follows the sender; `wireguard`/`quic` forces and verifies |
| `--wg-mtu` | | `1400` | WireGuard inner IP MTU (must match the sender; WireGuard only) |
| `--quic-cc` | | `bbr` | QUIC congestion control: `bbr`, `cubic`, or `newreno` (QUIC only) |
| `--quic-mtu` | | `1200` | Initial QUIC UDP payload, 1200–1452 (must match the sender) |

**Examples:**
```bash
# Receive to current directory
./hermod rx 3-apple-banana-cherry -s wss://relay.example.com:4376

# Save to a specific directory
./hermod rx 3-apple-banana-cherry -d ~/downloads/

# The receiver auto-detects QUIC by default; force/validate a transport if needed
./hermod rx 3-apple-banana-cherry --transport quic
./hermod rx 3-apple-banana-cherry --transport wireguard
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
6. The sender `prepare`s its chosen transport (QUIC by default) and wraps the
   opaque endpoint bytes with a small header: `"HE" + version + transport`. The
   bundle is AEAD-sealed with `K` and exchanged through the signaling relay — the
   server never learns it. The bundle also carries `SHA-256(token)` so each side
   can reject a peer from a different session. The receiver decrypts this first
   blob, reads the header, and only then prepares the matching transport and replies.
7. **QUIC** (default): the receiver generates an ephemeral self-signed certificate
   and the sender pins its SHA-256 from the encrypted bundle. The sender is the
   QUIC client (it initiates the connection outbound, like WireGuard's sender), the
   receiver is the QUIC server. HTTP/1.1 runs on one bidirectional QUIC stream
   opened by the receiver: tx `listen` = `accept_bi`, rx `dial` = `open_bi`.
   The receiver first sends a 32-byte session fingerprint (SHA-256 of the token);
   the sender rejects streams without it before handing the stream to HTTP. The
   receiver also sends a few dummy UDP packets to the sender's candidates to open
   stateful-firewall return paths. BBR is the default congestion control.
   **WireGuard** (`--transport wireguard`): each side generates an ephemeral X25519
   key pair and derives `PSK = HKDF-SHA256(K, channel_id, "hermod-wg-psk-v1")`. A
   WireGuard handshake runs over IPv6 UDP; then an in-process TCP/IP stack (smoltcp)
   carries HTTP/1.1.
8. In both cases the sender serves `GET /f/<token>`; the receiver pulls the whole
   payload while hashing SHA-256, then posts the digest to `POST /done`. The sender
   compares the digest with its own and only then completes the transfer. QUIC
   defaults to **BBR** and uses 64 MiB stream/connection windows, 8 MiB UDP socket
   buffers, and PMTUD.

## Tuning

- **Sender progress**: the `tx` progress bar uses the QUIC wire rate
  (`udp_tx.bytes`) for the speed/ETA display, while its position is still file
  bytes written locally. This means the speed can keep moving while the local
  64 MiB send window is blocked; it includes QUIC ACK/control/retransmit overhead.
- **QUIC throughput**: BBR is the default and is the recommended choice on lossy
  paths. Compare with `--quic-cc cubic` / `--quic-cc newreno` if needed. QUIC uses
  64 MiB stream and connection receive windows, a 64 MiB send window, 8 MiB UDP
  socket buffers, and PMTUD from 1200 to 1452. On PPPoE/lossy paths start with
  `--quic-mtu 1200`; on clean LAN/WAN paths PMTUD will raise it automatically.
- **WireGuard fallback throughput**: the inner TCP window defaults to 1 MiB. On a
  fast LAN you can raise it with `HERMOD_TCP_SOCKET_BUF` (bytes, e.g. `4194304`
  for 4 MiB) on both sides. Keep it small on lossy WAN paths — a large window
  amplifies bufferbloat.
- Check `--verbose info`'s `receive progress ... write_pct`: if it is high, the
  receiving disk is the bottleneck, not the network.

## Security properties

| Property | Mechanism |
|---|---|
| Confidentiality (WireGuard) | WireGuard with per-session ephemeral X25519 keys + a PSK derived from the CPace key |
| Confidentiality (QUIC) | TLS 1.3 with a per-session ephemeral self-signed certificate |
| Authentication | CPace PAKE — only someone with the transfer code can connect; QUIC pins the server certificate fingerprint from the CPace-encrypted bundle |
| Client authorization (QUIC) | A 32-byte session fingerprint preamble (SHA-256 of the one-time token) before HTTP; TLS client certificates are not required |
| Integrity | SHA-256 verified on both sides; the sender renames/releases only after the digest matches |
| Server cannot read data | Payload never touches the signaling server; the endpoint bundle is encrypted with the CPace key |
| Session isolation | Fresh ephemeral keys/PSK (WireGuard) or certificate (QUIC) per session; the one-time token is never sent to the signaling server |

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
  src/transport/       Transport trait + runtime dispatcher
    quic/              QUIC data plane (default): TLS pinning, BBR, streams
    wireguard/         WireGuard data plane (optional fallback)
      wg/              X25519 keys, boringtun wrapper, endpoint roaming
      stack/           smoltcp device/driver + AsyncRead/AsyncWrite bridge
    http/              shared transport-internal HTTP/1.1 (server + client)
  src/orchestrate.rs   tx/rx transfer main flow
hermod-cli/            thin CLI shell (clap) over hermod-core
```

## License

MIT — see [LICENSE](LICENSE).
