# Hermod

[English](README.md) · [简体中文](README.zh-CN.md)

**Send a file or text directly to another device with a short transfer code.**
Hermod is an encrypted, peer-to-peer **IPv4 LAN / IPv6** transfer tool written in Rust. Files
travel between your devices; the signaling server only helps them connect.

- **Windows GUI:** Flutter Material 3 with a Rust Core process, distributed through Microsoft Store as x64/ARM64 MSIXBundle.
- **Android:** Flutter runner exists for ongoing platform integration; no Android release package is currently published.
- **Linux CLI:** static musl binaries for terminals and automation.
- **Two protocols:** GUI sends with QUIC by default; CLI sends with WireGuard. Receivers automatically follow the sender.
- Files, text, cancellation, progress, adjustable bandwidth, server fingerprint pinning and SHA-256 verification.
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

**Use the same IPv4 LAN or reachable IPv6, and allow peer UDP traffic.** Both
QUIC and WireGuard support private/link-local IPv4 LAN addresses, including IPv4-only
networks. Pairing still requires the selected signaling server; offline discovery,
Internet IPv4 NAT traversal and file relays are not provided. A network address
by itself does not guarantee that your router or firewall permits peer traffic.
The default signaling server is `wss://oracle.stariverfeel.eu.org:4376`; its SPKI
fingerprint is built in. A transfer code becomes usable after the server allocates it.

WSL NAT commonly lacks usable IPv6. On supported Windows 11/WSL systems, enable
`networkingMode=mirrored` in the Windows user profile's `.wslconfig`, then run
`wsl --shutdown` from Windows. Your Windows network must already have IPv6. Assigning
an arbitrary IPv6 address does not provide Internet routing. See [network troubleshooting](NETWORK_PERFORMANCE.md).

## GUI: three steps

1. On the sender, choose **File** and select a document, or choose **Text** and enter a message. Start sending.
2. Share the complete code privately. On the receiver, enter it under **Receive** and connect.
3. Wait for **SHA-256 verified**. Open **Inbox → Save a copy**, or choose a default export folder in Settings.

Verified files remain in a private inbox until deleted. Failed exports do not delete that copy. Text arrives as a text file. Cancellation or process termination discards unfinished transfers; there is no resume. Rust Core owns transfer state while Flutter remains presentation-only.

**Language:** the Windows GUI supports English and Simplified Chinese.
The default follows your system language. Choose **Settings → Language → English /
简体中文 / System**, then **Save settings** to remember it. Language changes apply
immediately; original technical logs and unknown diagnostic details remain available.

Settings cover send/receive protocol, MTU, congestion control, bandwidth, code length,
server, fingerprint trust, receive folder, log level, reduced notifications, theme and language.
They persist locally. Each new transfer takes a settings snapshot; saving changes does
not reconfigure a running connection. New GUI installations use **QUIC, Auto receive
and unlimited speed (0)**. Existing preferences are preserved; Settings offers **Use
new defaults**, followed by Save settings. You can still set a bandwidth cap to keep
capacity available for other applications.

The encrypted pairing exchange announces the filename, type and size before payload
transfer. Receivers show the total immediately and reject a conflicting HTTP length
or metadata. Older peers remain compatible over IPv6; their size is learned from
HTTP headers. IPv4 LAN transfers require upgraded clients on both devices.

## Installation and upgrades

**Windows:** install **HermodTransfer** from [Microsoft Store](https://apps.microsoft.com/detail/9PMPWN3HMR3M)
(publisher **FeelLiao**, Store ID **9PMPWN3HMR3M**). Version-tag builds automatically submit
the validated x64/ARM64 bundle after GitHub Release succeeds. Microsoft signs certified
updates and delivers the matching architecture. Windows packages stay in Actions and
Microsoft Store, outside GitHub Release attachments. Before migrating from `Hermod.Transfer.Test`,
export important inbox files: the Store package uses a different identity.
See [Store submission details](APP_BUILD.md#microsoft-store-packages).

**Linux:** Ubuntu 22.04 or newer compatible desktops are the build baseline:

For Windows upgrades, keep the Store package identity unchanged. Uninstalling can remove private application data, so export inbox files before uninstalling a package installed under another identity.

## CLI quick start

```bash
chmod +x hermod-linux-x64-musl
./hermod-linux-x64-musl tx ./archive.zip --limit-mbps 8
# On the receiver, replace CODE with the sender's complete code:
./hermod-linux-x64-musl rx CODE --destination ./received
```

Or install the binary as `hermod` and use:

```bash
hermod tx 'Hello from Hermod'
printf 'Read from stdin\n' | hermod tx -
hermod tx ./archive.zip --transport quic --mtu 1280 --congestion bbr
hermod rx CODE --transport wireguard --limit-mbps 8
hermod --help
hermod tx --help
```

CLI has no default bandwidth cap (`0` means unlimited). Sending defaults to WireGuard;
receiving defaults to `auto`. Keep both peers open until the digest verdict completes.

| Option | Meaning |
| --- | --- |
| `--transport wireguard\|quic` | Sender protocol; receiver also accepts `auto` |
| `--mtu auto\|NUMBER` | One MTU setting for the selected protocol |
| `--congestion auto\|cubic\|reno\|bbr\|newreno` | Only algorithms supported by the selected protocol are accepted |
| `--limit-mbps NUMBER` | Decimal megabits/s of payload, 0 unlimited; lower sender/receiver cap wins |
| `--server URL` | Signaling server (must be trusted) |
| `--words NUMBER` | Sender code length, 1–32 words, default 3 |
| `--destination PATH` | Receiver output directory |
| `--verbose LEVEL`, `--quiet` | Global logging level and reduced status output |

**MTU semantics:** WireGuard uses the inner IP MTU (auto: 1400, accepted 576–65488).
QUIC uses a UDP payload ceiling (1200–1452); auto begins at 1200 and probes up to 1452.
An explicit QUIC cap is never exceeded by discovery. Larger values do not automatically
make WireGuard faster: leave space for outer IPv6/UDP/WireGuard overhead.

**Congestion control:** `auto` means CUBIC on WireGuard and BBR on QUIC. WireGuard
supports CUBIC/Reno; QUIC supports BBR/CUBIC/NewReno. Unsupported combinations fail
explicitly, including after an auto receiver learns the sender's protocol.

**Custom server trust:** independently obtain the server's SHA-256 SPKI fingerprint,
then use `hermod trust SERVER --fingerprint HEX` (see `hermod trust --help`). The GUI
provides Fetch fingerprint and an explicit confirmation; fetching alone is not identity
verification. Each client profile keeps **one trusted server**. Successful verification replaces
the old trust and selects the new server in one operation; failed verification
retains the previous configuration. Old trust lists migrate to the selected server.
`--server` must match that single trusted address. CLI config is
`$XDG_CONFIG_HOME/hermod/config.yaml`, or `~/.config/hermod/config.yaml` by default;
GUI config and verified inbox are in the platform's application data directory.

## Migrating from 0.3.x

| Removed | Replacement |
| --- | --- |
| `--wg-mtu N` | `--transport wireguard --mtu N` |
| `--quic-mtu N` | `--transport quic --mtu N` (now a hard discovery ceiling) |
| `--quic-cc ALGORITHM` | `--transport quic --congestion ALGORITHM` |
| `--listen ADDRESS` / bind address | Automatic dual-stack binding with IPv4 fallback; no public binding control |
| `serve` placeholder | Removed; run a compatible signaling server separately |

Windows uses Flutter Material 3. The GUI loads `hermod_flutter_bridge.dll` over flutter_rust_bridge (FFI), whose `gui_api` facade owns settings, jobs and the private inbox while relying on `hermod-core` for all transport work; private inbox paths never cross into Flutter.

## Troubleshooting and development

- **Code cannot connect:** check both peers' IPv4 LAN/IPv6 routing, signaling reachability,
  UDP firewall rules, complete code and compatible forced protocols.
- **Browsing slows while sending:** lower `--limit-mbps` / GUI bandwidth. A saturated
  upload can queue browsing requests; see [performance and latency](NETWORK_PERFORMANCE.md).
- **Export failed:** keep the verified inbox copy, choose a writable folder again, then retry Save a copy.
- **Windows MSIXBundle will not install:** the GitHub bundle is an unsigned Store submission
  packages. Install the Store-signed version after it has been approved and published.

Actions are split: `CI` checks branch pushes and pull requests. `Build and release`
reuses those checks before packaging; only a `v*` tag push publishes a Release.
Manual builds produce Actions artifacts without publishing.

[Build and release guide](APP_BUILD.md) · [Architecture and contributor guide](AGENTS.md) ·
[Changelog](CHANGELOG.md) · [Privacy policy / 隐私政策](PRIVACY.md) · [License](LICENSE)

Inspired by [Magic Wormhole](https://github.com/magic-wormhole/magic-wormhole).
The protocol uses CPace, authenticated endpoint exchange and transport encryption;
the Go implementation shares the signaling and transfer-code formats.
