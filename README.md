# Hermod

[English](README.md) · [简体中文](README.zh-CN.md)

**Send a file or text directly to another device with a short transfer code.**
Hermod is an encrypted, peer-to-peer **IPv6** transfer tool written in Rust. Files
travel between your devices; the signaling server only helps them connect.

- **Desktop GUI:** Tauri 2, React and Microsoft's official Fluent UI React v9 (Fluent 2).
- **Android GUI:** Tauri with Google’s official Material Design 3 Web components, dynamic colors and adaptive navigation.
- **Linux CLI:** static musl binaries for terminals and automation.
- **Two protocols:** WireGuard by default when sending; QUIC is available. Receivers automatically follow the sender.
- Files, text, cancellation, progress, adjustable bandwidth, server fingerprint pinning and SHA-256 verification.
- Runs in userspace: no VPN configuration, TUN interface or administrator privileges for transfers.

## Download

Get programs from [GitHub Releases](https://github.com/FeelLiao/hermod-rs/releases/latest).
Choose your device's CPU architecture, not the architecture of the sender.

| Device | Files to download |
| --- | --- |
| Windows 10 2004+ / Windows 11, x64 | `hermod-windows-x64.msix` (Store submission only) |
| Windows on ARM64 | `hermod-windows-arm64.msix` (Store submission only) |
| Linux desktop x64 / ARM64 | `hermod-linux-x64.AppImage` / `hermod-linux-arm64.AppImage` |
| Linux terminal x64 / ARM64 | `hermod-linux-x64-musl` / `hermod-linux-arm64-musl` |
| Android 14+ ARM64 | `hermod-android-arm64.apk` |

There are no macOS, Windows CLI or Android CLI release builds. Release attachments
contain only seven packages/programs; Windows MSIX files are unsigned Store uploads,
not direct-install downloads. No certificates or installation scripts are published.
GitHub also provides automatic source archives. Android CI builds and signs the APK
and checks the package; it does not run Android emulator or UI tests.

## Before connecting

**Both peers need reachable IPv6 and permission to exchange UDP traffic.** IPv4-only
networks, IPv4 NAT traversal and relay transfers are not supported. An IPv6 address
by itself does not guarantee that your router or firewall permits incoming traffic.
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

Verified files remain in a private inbox until deleted. Failed exports and expired
Android folder permissions do not delete that copy. Text arrives as a text file.
Cancellation or process termination discards unfinished transfers; there is no resume.
Android uses system document permissions, never unrestricted storage access. Rust owns the transfer and a foreground service provides progress and cancellation; activity recreation does not restart it.
On Android 17, grant Nearby devices/local network access for peer transfers. Notification
permission enables the visible progress notification; denying it does not grant storage access.

**Language:** the desktop and Android GUI support English and Simplified Chinese.
The default follows your system language. Choose **Settings → Language → English /
简体中文 / System**, then **Save settings** to remember it. Language changes apply
immediately; original technical logs and unknown diagnostic details remain available.

Settings cover send/receive protocol, MTU, congestion control, bandwidth, code length,
server, fingerprint trust, receive folder, log level, reduced notifications, theme and language.
They persist locally. Each new transfer takes a settings snapshot; saving changes does
not reconfigure a running connection. The default GUI cap is **8 Mbps**: adjust it to
your uplink, typically starting at 70–85% of measured upload capacity.

## Installation and upgrades

**Windows:** install Hermod from Microsoft Store after the listing is approved and
published (publisher **FeelLiao**, Store ID **9PMPWN3HMR3M**). The x64 and ARM64 MSIX
files on GitHub are for the maintainer to upload together to Partner Center's
**Packages** page. Microsoft signs the certified packages for installation.
The unsigned GitHub files cannot be installed by double-clicking; no test certificate
or installation script is provided. Before migrating from `Hermod.Transfer.Test`,
export important inbox files: the Store package uses a different identity.
See [Store submission details](APP_BUILD.md#microsoft-store-packages).

**Linux:** Ubuntu 22.04 or newer compatible desktops are the build baseline:

```bash
chmod +x hermod-linux-x64.AppImage
./hermod-linux-x64.AppImage
# If FUSE is unavailable:
APPIMAGE_EXTRACT_AND_RUN=1 ./hermod-linux-x64.AppImage
```

**Android:** allow APK installation from your download application, then open the APK.
Only ARM64 is published. Minimum SDK is 34; compile/target SDK is 37. Keep the same
signing keys when upgrading. For old releases signed with a temporary key, **export
all inbox files before uninstalling** and installing v0.4.3. The same precaution
applies to Windows packages with a different prior signing identity. Uninstalling can
remove private application data.

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
verification. Trust and server selection are separate settings. CLI config is
`$XDG_CONFIG_HOME/hermod/config.yaml`, or `~/.config/hermod/config.yaml` by default;
GUI config and verified inbox are in the platform's application data directory.

## Migrating from 0.3.x

| Removed | Replacement |
| --- | --- |
| `--wg-mtu N` | `--transport wireguard --mtu N` |
| `--quic-mtu N` | `--transport quic --mtu N` (now a hard discovery ceiling) |
| `--quic-cc ALGORITHM` | `--transport quic --congestion ALGORITHM` |
| `--listen ADDRESS` / bind address | Automatic `[::]:0`; no public binding control |
| `serve` placeholder | Removed; run a compatible signaling server separately |

Desktop uses Fluent 2; Android uses Material Design 3 Web. Both use the same React
pages, settings and transfer engine inside `hermod-app`. Only system file access,
permissions and foreground notifications need Android native adapters. Old desktop bandwidth/theme preferences
are migrated on first launch when possible; private verified inbox data is retained.

## Troubleshooting and development

- **Code cannot connect:** check both peers' IPv6 routing, signaling reachability,
  UDP firewall rules, complete code and compatible forced protocols.
- **Browsing slows while sending:** lower `--limit-mbps` / GUI bandwidth. A saturated
  upload can queue browsing requests; see [performance and latency](NETWORK_PERFORMANCE.md).
- **Export failed:** keep the verified inbox copy, choose a writable folder again,
  then retry Save a copy. Regrant Android folder access if necessary.
- **Windows MSIX will not install:** GitHub MSIX files are unsigned Store submission
  packages. Install the Store-signed version after it has been approved and published.

Actions are split: `CI` checks branch pushes and pull requests. `Build and release`
reuses those checks before packaging; only a `v*` tag push publishes a Release.
Manual builds produce Actions artifacts without publishing.

[Build and release guide](APP_BUILD.md) · [Architecture and contributor guide](AGENTS.md) ·
[Changelog](CHANGELOG.md) · [License](LICENSE)

Inspired by [Magic Wormhole](https://github.com/magic-wormhole/magic-wormhole).
The protocol uses CPace, authenticated endpoint exchange and transport encryption;
the Go implementation shares the signaling and transfer-code formats.
