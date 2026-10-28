# Upload saturation and responsiveness

In unlimited mode the sender continuously supplies data to QUIC, whose BBR
controller probes for available capacity. It has a 64 MiB application window
and requests 8 MiB UDP socket buffers. WireGuard uses CUBIC by default, but an unlimited transfer also reserves no
application bandwidth for other traffic. These settings favor bulk throughput, but reserve no capacity
for browsers or other applications sharing the uplink.

When the bottleneck queue fills, small HTTP requests and TCP acknowledgements
wait behind the upload. This is consistent with the reported sender-side
slowdown. A large application window is not itself proof of router queue buildup;
CPU load, Wi-Fi and router behavior can also contribute. See
[RFC 7567 on queue management and latency](https://www.rfc-editor.org/rfc/rfc7567.html).

## Shared bandwidth and transport settings

`--limit-mbps` paces file payload **before** writing to QUIC or WireGuard. Each
chunk represents at most 20 ms of the configured budget (maximum 64 KiB).
Time spent waiting for storage does not accumulate credits for a later burst.
The receiver can declare its own cap in the encrypted HTTP request, and an
upgraded sender applies the lower cap. The receiver also limits its local read
rate for older senders, though an old sender can fill its transport buffers first.

With an explicit cap, the QUIC application window becomes approximately 250 ms
of the configured rate, clamped to 256 KiB–64 MiB. Unlimited mode preserves the
original window and congestion controller. BBR, CUBIC and NewReno all remain
available; switching to CUBIC alone does not reserve upload capacity.

Mbps means decimal megabits per second: 8 Mbps is 1,000,000 payload bytes/s.
Encryption, IP headers and retransmissions are additional traffic. Start at
70–85% of **measured upload capacity**, then compare browsing latency. The GUI
starts unlimited, as does the CLI (0). Set an explicit cap when needed; the app
does not automatically estimate uplink capacity. Limits apply per transfer.

## Reproduction

Run `bash scripts/latency-check.sh` on Linux with Rust, Python 3, iproute2, ping
and permission to create network namespaces. All shaping takes place in an
isolated namespace; no host traffic is shaped. The example transfers 8 MiB over
IPv6 QUIC and verifies SHA-256, while ping shares a simulated 10 Mbps link with
10 ms delay in each direction and a 1,000-packet queue.

Observed in this WSL environment on 2026-10-24:

| Mode | Payload throughput | Mean ping | 95th percentile | Maximum |
|---|---:|---:|---:|---:|
| Unlimited | 7.853 Mbps | 1191.9 ms | 2514.0 ms | 2520.0 ms |
| 7 Mbps payload cap | 6.364 Mbps | 33.6 ms | 44.9 ms | 46.5 ms |

Both runs verified the same SHA-256. These are controlled local measurements,
not measurements of the user's Internet connection. The shared simulated link
carries data and acknowledgements in both directions. Actual gains depend on
uplink capacity, competing traffic and queue management. Router SQM/AQM can
additionally help all applications; Hermod does not change router settings.

## Protocol settings

GUI sending defaults to QUIC; CLI sending defaults to WireGuard. Receiving defaults to Auto. `--congestion auto`
selects CUBIC for WireGuard or BBR for QUIC. WireGuard supports CUBIC/Reno; QUIC
supports BBR/CUBIC/NewReno. Selecting an unsupported algorithm fails explicitly.
The single `--mtu` option controls the active protocol: WireGuard inner IP MTU
(default 1400), or QUIC UDP payload ceiling (1200–1452). QUIC starts at 1200 and
never probes above an explicit ceiling. Neither MTU nor congestion control replaces
a bandwidth budget. Payload limits exclude outer headers and retransmissions.

Desktop and Android share settings and take a snapshot for each transfer. The GUI
starts unlimited; CLI also starts unlimited. Changing settings while transferring affects
the next job. In-memory queues for document callbacks are limited to two 64 KiB chunks;
unknown-length SAF documents stage to private storage before advertising a code.

## IPv6 and WSL

IPv4 LAN direct transfer is supported by both transports; pairing still uses the
selected signaling server. No offline discovery, Internet IPv4 NAT traversal or relay
is included. For Internet transfers, both devices need routed IPv6 and UDP firewall
permission. Binding is automatic on `[::]:0`; the removed `--listen` flag is not a
connectivity fix. Link-local-only and fabricated IPv6 addresses cannot reach remote peers.

On supported Windows 11 and WSL, use the Windows profile `.wslconfig`:

```ini
[wsl2]
networkingMode=mirrored
dnsTunneling=true
```

Run `wsl --shutdown` from Windows after saving. Verify Windows has IPv6 first, then
check `ip -6 addr`, `ip -6 route` and the signaling host's DNS/port from WSL. Mirrored
networking still obeys Windows/Hyper-V and router firewall rules. See Microsoft's
[WSL networking guide](https://learn.microsoft.com/windows/wsl/networking).
Do not disable all firewalls or assign arbitrary public addresses as a workaround.
