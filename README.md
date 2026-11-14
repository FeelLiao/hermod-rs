# Hermod

[English](README.md) · [简体中文](README.zh-CN.md)

**Send a file or text directly to another device with a short transfer code.**
Hermod is an encrypted, peer-to-peer **IPv4 LAN / IPv6** transfer tool written in Rust. Files
travel between your devices; the signaling server only helps them connect.

- **Windows GUI:** Flutter Material 3 with a Rust Core process, distributed through Microsoft Store as x64/ARM64 MSIXBundle.
- **Android:** Flutter runner exists for ongoing platform integration; no Android release package is currently published.
- **Linux CLI:** static musl binaries for terminals and automation.
- **One data-plane protocol:** QUIC (TLS 1.3, BBR by default) for both GUI and CLI; receivers follow automatically.
- Files, text, cancellation, progress, server fingerprint pinning and SHA-256 verification.
- Runs in userspace: no VPN configuration, TUN interface or administrator privileges for transfers.

## Download

Get programs from [GitHub Releases](https://github.com/FeelLiao/hermod-rs/releases/latest).
Choose your device's CPU architecture, not the architecture of the sender.

| Device | Files to download |
| --- | --- |
| Windows 10 2004+ / Windows 11, x64 / ARM64 | [HermodTransfer in Microsoft Store](https://apps.microsoft.com/detail/9PMPWN3HMR3M) |
| Linux terminal x64 / ARM64 | `hermod-linux-x64-musl` / `hermod-linux-arm64-musl` |

There are no macOS, Linux GUI, Windows CLI, or Android release builds. Release attachments contain only the two Linux CLI programs. Windows packages are submitted directly to Microsoft Store. No certificates or installation scripts are published. GitHub also provides automatic source archives.

## Before connecting

**Use the same IPv4 LAN or reachable IPv6, and allow peer UDP traffic.**
QUIC supports private/link-local IPv4 LAN addresses, including IPv4-only
networks. Pairing still requires the selected signaling server; offline discovery,
Internet IPv4 NAT traversal and file relays are not provided. A network address
by itself does not guarantee that your router or firewall permits peer traffic.
The default signaling server is `wss://oracle.stariverfeel.eu.org:4376`; its SPKI
fingerprint is built in. A transfer code becomes usable after the server allocates it.

WSL NAT commonly lacks usable IPv6. On supported Windows 11/WSL systems, enable
`networkingMode=mirrored` in the Windows user profile's `.wslconfig`, then run
`wsl --shutdown` from Windows. Your Windows network must already have IPv6. Assigning
an arbitrary IPv6 address does not provide Internet routing.

## GUI: three steps

1. On the sender, choose **File** and select a document, or choose **Text** and enter a message. Start sending.
2. Share the complete code privately. On the receiver, enter it under **Receive** and connect.
3. Wait for **SHA-256 verified**. Open **Inbox → Save a copy**, or choose a default export folder in Settings.

Verified files remain in a private inbox until deleted. Failed exports do not delete that copy. Text arrives as a text file. Cancellation or process termination discards unfinished transfers; there is no resume. Rust Core owns transfer state while Flutter remains presentation-only.

**Language:** the Windows GUI supports English and Simplified Chinese.
The default follows your system language. Choose **Settings → Language → English /
简体中文 / System**, then **Save settings** to remember it. Language changes apply
immediately; original technical logs and unknown diagnostic details remain available.

GUI settings persist locally, and each new transfer takes a settings snapshot;
saving changes does not reconfigure a running connection. The CLI tuning work below
does not change GUI/bridge settings or promise compatibility with old `compatible`
profile values. Its new tuning flags are not persisted or read from YAML.

The encrypted pairing exchange announces filename, type and size before payload
transfer. Current payload protocol v3 runs directly on QUIC streams and verifies
SHA-256; HTTP and WireGuard payloads are no longer supported. Use upgraded clients
on both devices; incompatible protocol versions fail explicitly.

## Installation and upgrades

**Windows:** install **HermodTransfer** from [Microsoft Store](https://apps.microsoft.com/detail/9PMPWN3HMR3M)
(publisher **FeelLiao**, Store ID **9PMPWN3HMR3M**). Version-tag builds automatically submit
the validated x64/ARM64 bundle after GitHub Release succeeds. Microsoft signs certified
updates and delivers the matching architecture. Windows packages stay in Actions and
Microsoft Store, outside GitHub Release attachments. Before migrating from `Hermod.Transfer.Test`,
export important inbox files: the Store package uses a different identity.
See [Store submission details](APP_BUILD.md#microsoft-store-identity).

**Linux:** download the musl CLI binary matching your CPU, then make it executable as shown below. Source builds require Rust 1.99 or newer and native C/C++ build tools.

For Windows upgrades, keep the Store package identity unchanged. Uninstalling can remove private application data, so export inbox files before uninstalling a package installed under another identity.

## Build the CLI and server

From the repository root, with the pinned Rust toolchain and native C/C++ tools installed:

```text
cargo build -p hermod-cli --release --locked
```

Windows produces `target/release/hermod.exe`; Linux produces `target/release/hermod`.
Both client commands and `serve` use this same executable. Verify the binary you run:

```powershell
# Windows
.\target\release\hermod.exe --version
```

```bash
# Linux
./target/release/hermod --version
```

This source version reports `hermod 0.5.0`. Windows CLI source builds are supported
for local use even though no Windows CLI release asset is published. Linux requires
a native linker such as `cc`; Rust alone is not sufficient. See [build details](APP_BUILD.md).

## CLI device pairing and remarks

Device commands are available in 0.5.0. Both peers must select the same trusted
signaling server. ID calls require an updated Rust server; the built-in server's
deployment version is not guaranteed by the client version.

First establish long-term trust explicitly on **both** sides:

```text
# Sender
hermod tx ./hello.txt --pair
# Receiver: use the complete transfer code printed by the sender
hermod rx CODE --pair
```

Ordinary `tx` / `rx CODE` transfers do not save contacts. Pairing confirmation and
file-transfer success are separate results: a later QUIC/file failure need not undo
a confirmed pairing. Repeating an existing matching pairing does not rotate its
secret or overwrite its local remark.

```text
hermod device id
hermod device list
hermod device rename DEVICE_ID "My laptop"
hermod device remove DEVICE_ID
```

A device ID is 32 lowercase hex characters derived from its public key. `rename`
sets a local remark, not a new ID or a name on the remote device. Remarks may repeat,
but `--from` requires an exact, unambiguous match; use the full ID when ambiguous.
A canonical ID is resolved as an ID first, not retried as a remark if absent.

For later transfers:

```text
# Sender must actively select this transfer's contents
hermod tx ./archive.zip
# Receiver: select by ID or local remark
hermod rx --from DEVICE_ID -d ./received
hermod rx --from "My laptop" -d ./received
```

`rx CODE` and `rx --from DEVICE` are mutually exclusive; `--from` cannot use `--pair`.
There is no background client daemon, remote file browser or arbitrary file pull.
One local identity can publish only one sending task at a time; another `tx` fails busy.
The sender keeps a code entry and an independent device-control connection; only an
authenticated candidate wins. Unknown or invalid ID candidates do not invalidate the
code. If device control is unsupported or disconnects, code transfer can remain
available; use `--verbose warning` for that diagnostic. ID authentication never silently
falls back to code authentication. Explicit pairing requires support and consent on both ends.

## Configuration and private files

| Platform | CLI configuration root |
| --- | --- |
| Windows | `%APPDATA%\Hermod` |
| Linux with an absolute `XDG_CONFIG_HOME` | `$XDG_CONFIG_HOME/hermod` |
| Linux otherwise | `~/.config/hermod` |

If the platform's configuration location cannot be determined, the implementation
falls back to a temporary directory; that is not a durable deployment location.
The configuration root is independent of the executable and current working directory.

```text
config root/
├── config.yaml             # selected server and its SPKI trust
├── device-state/
│   ├── device.json         # long-term identity, including PRIVATE KEY
│   ├── contacts.json       # pairing secrets, remarks, pending/active states
│   └── *.lock              # identity/contact transactions and sending lease
└── server/                 # created by serve
    ├── server-cert.pem     # public TLS certificate
    ├── server-key.der      # PRIVATE KEY, PKCS#8 DER
    └── server-cert.json    # certificate expiry metadata
```

Missing `config.yaml` uses built-in defaults; successful `trust` saves it. Its schema
contains `trust_version`, `server_url`, `trusted_servers` (currently one selected
server), and retained `tls_configuration` fields. Legacy PEM fields may be preserved;
new Rust server credentials live in `server/`, not those YAML fields. Tuning options
and receive destinations are not persisted. The global device ID and contacts remain
stable across server changes, but peers must meet on the same server; a stable ID
also permits correlation across servers.

Treat identity, contacts and server private keys as secrets: do not commit or share
them. Linux device storage checks owner-only 0700/0600 permissions; Windows device
storage sets and verifies private permissions for the current user and SYSTEM via
system PowerShell. No DPAPI/Keychain encryption is implemented. Parent directories
must be trusted; this is not protection against a malicious process running as you.
Invalid identity data or unsafe permissions cause errors, never silent identity reset.

Matching pending pairing states can be reconciled by explicitly repeating `--pair`.
An active pairing missing on the other side, or conflicting records, requires removing
the old contact on both sides and pairing again. Deleting a contact revokes only local
trust; the other device is not remotely erased and cannot automatically restore it.

## Run a Rust signaling server

```text
hermod serve -p 4376 --verbose info
# Restrict listeners if needed
hermod -4 serve -p 4376 --verbose info
hermod -6 serve -p 4376 --verbose info
```

Allow TCP 4376 for TLS/WSS and UDP 4376 for address reflection, plus peer UDP traffic
between clients. The service listens dual-stack by default. Port precedence is explicit
`--port` > `HERMOD_PORT` > 4376; `--port 0` selects an ephemeral port for local testing.
Persistent deployments should use a fixed port and preserve the `server/` directory.
Deleting its private key changes the SPKI fingerprint and requires clients to trust it again.
The server checks certificate renewal on startup and normally reuses the existing key.

Read the startup fingerprint and verify it independently on each client:

```text
hermod trust your-server.example.com:4376 --fingerprint HEX
```

Device registration proves possession of a private key; paired-device trust is still
checked end-to-end. Device routing state is in memory, while the server can observe
IDs, call relationships and source addresses. This service is not a file relay and does
not solve every NAT/firewall reachability problem. Local integration tests are not
public-network endurance or load certification. Supervise a production process and
back up its private keys without exposing them.

## CLI quick start

```bash
chmod +x hermod-linux-x64-musl
./hermod-linux-x64-musl tx ./archive.zip
# On the receiver, replace CODE with the sender's complete code:
./hermod-linux-x64-musl rx CODE -d ./received
```

Or install the binary as `hermod` and use:

```bash
hermod tx 'Hello from Hermod'
printf 'Read from stdin\n' | hermod tx -
hermod -4 tx ./archive.zip --mtu 1280 --congestion bbr
hermod rx CODE --streams 4 --output ./received
hermod serve -p 4376
hermod --help
hermod tx --help
```

## CLI transport options

Available in 0.5.0. QUIC is the only data plane; receiving follows it automatically. There is no payload
rate limiter. Keep both peers open until the digest verdict completes. Tuning applies
only to this invocation: explicit values override built-in defaults, unspecified values
use defaults. Tuning is neither persisted nor read from YAML. Server trust configuration
still persists separately.

| Option | Default / meaning |
| --- | --- |
| `-4/--ipv4`, `-6/--ipv6` | Mutually exclusive; strictly restrict networking in tx/rx/trust/serve, including device control; no cross-family fallback |
| `-m/--mtu auto\|NUMBER` | auto; QUIC UDP payload cap 1200–1452 bytes; auto probes from 1200 to 1452 |
| `-c/--congestion auto\|bbr\|cubic\|newreno` | bbr; auto also selects BBR |
| `-P/--profile auto\|lan\|wan` | auto; no compatible alias |
| `--streams auto\|1..8` | auto; rx selects actual concurrency, subject to source capability and file size |
| `--chunk SIZE` | 1MiB; 256KiB–4MiB |
| `--window SIZE` | 64MiB per stream; 8MiB–256MiB; connection receive/send budget = window × 8 |
| `--udp-buffer SIZE` | 16MiB; 1MiB–64MiB requested socket buffer |
| `--slice-size SIZE` | 4MiB; 4MiB–64MiB slice target |
| `--prefetch NUMBER` | 16; 4–64 queued chunks |
| `--keepalive SECONDS` | 10; 0–60, 0 disables keepalive |
| `--idle-timeout SECONDS` | 30; 10–300 and strictly greater than keepalive |
| rx `-d/--output DIR` | `.`; output directory |
| serve `-p/--port PORT` | Numeric 0–65535; CLI > HERMOD_PORT > 4376 |
| `--verbose LEVEL`, `-q/--quiet` | Global log level (default none) and reduced status output |

Enum values are case-insensitive. Sizes accept integer bytes or integer KiB/MiB only
(e.g. `1048576`, `1024KiB`, `1MiB`), not decimals, MB or embedded spaces. Invalid values
fail before networking. All data-plane flags (mtu, congestion, profile and the eight
tuning options) are errors when explicitly passed to serve/trust/device, even with default values.

The receiver owns the stream plan: tx tuning cannot force rx concurrency. Auto profiles
use peer address classification, with a valid RTT ≤25ms additionally required for LAN
multi-stream selection when RTT is available. Keepalive/idle timeout configure QUIC;
idle-timeout also sets the CLI payload no-progress watchdog, not the separate handshake
budget. Larger windows, chunks and prefetch queues consume more resources, not guaranteed
throughput. The default connection window budget is 512MiB (up to 2GiB at the largest
setting); this is not an up-front allocation. A chunk × prefetch queue alone budgets
about 1–256MiB, excluding other buffers. Measure throughput, latency and memory on
your actual network; no before/after speedup has been established.

**Custom server trust:** independently obtain the server's SHA-256 SPKI fingerprint,
then use `hermod trust SERVER --fingerprint HEX` (see `hermod trust --help`). The GUI
provides Fetch fingerprint and an explicit confirmation; fetching alone is not identity
verification. Each client profile keeps **one trusted server**. Successful verification replaces
the old trust and selects the new server in one operation; failed verification
retains the previous configuration. Old trust lists migrate to the selected server.
tx/rx use that selected server; there is no per-transfer `--server` override. CLI config is
`$XDG_CONFIG_HOME/hermod/config.yaml`, or `~/.config/hermod/config.yaml` by default;
GUI config and verified inbox are in the platform's application data directory.

## Migrating from 0.3.x

| Removed | Replacement |
| --- | --- |
| `--wg-mtu`, `--transport`, `--limit-mbps` | Removed; QUIC only, no payload rate limiter |
| `--quic-mtu N` | `--mtu N`, a hard discovery ceiling |
| `--quic-cc ALGORITHM` | `--congestion ALGORITHM` |
| tx/rx `--server`, tx `--words` | Select server with trust; CLI codes are fixed at three words |
| rx `--destination DIR` | `-d/--output DIR` |
| `--profile compatible` | Use wan as appropriate; old value and alias are removed |
| serve `-l/--listen`, HERMOD_LISTEN | `serve -p/--port`, HERMOD_PORT; CLI > environment > 4376 |

`serve` now runs the signaling/NAT helper service, with dual-stack listening by default;
use global -4 or -6 to restrict its family. This is not a file relay.

Windows uses Flutter Material 3. The GUI loads `hermod_flutter_bridge.dll` over flutter_rust_bridge (FFI), whose `gui_api` facade owns settings, jobs and the private inbox while relying on `hermod-core` for all transport work; private inbox paths never cross into Flutter.

## Troubleshooting and development

- **Code cannot connect:** check both peers' IPv4 LAN/IPv6 routing, signaling reachability,
  UDP firewall rules, complete code, client versions and any -4/-6 restriction.
- **Device ID cannot connect:** check the updated Rust server, the sender's active
  `tx` task, and matching active pairings on both ends. A device ID is not a password.
- **Browsing slows while sending:** a saturated upload can queue browsing requests.
  Hermod has no payload rate limiter; consider router SQM/AQM or external traffic shaping.
- **Export failed:** keep the verified inbox copy, choose a writable folder again, then retry Save a copy.
- **Windows MSIXBundle will not install:** the GitHub bundle is an unsigned Store submission
  packages. Install the Store-signed version after it has been approved and published.

Actions are split: `CI` checks branch pushes and pull requests. `Build and release`
reuses those checks before packaging; only a `v*` tag push publishes a Release.
Manual builds produce Actions artifacts without publishing.

[Build and release guide](APP_BUILD.md) · [Core architecture](core/README.md) ·
[Changelog](CHANGELOG.md) · [Privacy policy / 隐私政策](PRIVACY.md) · [License](LICENSE)

Inspired by [Magic Wormhole](https://github.com/magic-wormhole/magic-wormhole).
The protocol uses CPace, authenticated endpoint exchange and transport encryption;
the legacy Go implementation shares the original signaling and transfer-code formats,
not the new device-control protocol or necessarily the current payload version.
