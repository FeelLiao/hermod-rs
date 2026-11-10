# Throughput tuning and responsiveness

Hermod moves payload end-to-end over QUIC (UDP). The sender continuously
supplies data to the QUIC connection, whose BBR controller probes for
available capacity. Defaults favor bulk throughput:

- 64 MiB stream / connection receive windows and 64 MiB send window.
- 16 MiB UDP socket buffer requests (failures are logged, never fatal).
- PMTUD up to the 1452-byte QUIC UDP payload ceiling (1500-byte Ethernet).
- ACK Frequency with a 5 ms max ACK delay for faster loss/RTT feedback.
- BBR by default; it treats random loss as noise instead of congestion,
  which is what usually collapses CUBIC on Wi-Fi.

## Link profiles (data-plane concurrency)

The receiver picks the data-plane shape per transfer (`--profile` on the
CLI, 链路档位 in the GUI settings):

- `auto` — probe the smoothed RTT after the handshake: at most 25 ms
  selects the LAN shape, anything else stays on the compatible shape.
- `lan` — force the LAN shape: the payload is split into slices of at
  least 4 MiB (up to 256 slices, each stream-serialized) and up to 8
  streams send slices in parallel. One loss event blocks reassembly of
  one slice instead of the whole single stream, which keeps the Wi-Fi
  link full while retransmissions recover. The receiver writes slices at
  their file offset and hashes slices in order by reading them back
  through the page cache.
- `compatible` — single stream with a 1 MiB chunk pipeline and a 16-chunk
  read-ahead; the receiver streams sequentially to disk.

Multi-stream requires the sender's payload source to be seekable (a real
file path). The capability is announced in the encrypted endpoint bundle
(`multi_stream`); stdin, text and document streams fall back to the single
stream shape. A protocol-version mismatch fails explicitly with an
upgrade hint instead of misparsing.

On Wi-Fi, expect the radio, not the transport, to be the ceiling: a 2x2
Wi-Fi 6 (80 MHz) link tops out well below gigabit wire speed, and traffic
between two Wi-Fi devices crosses the air twice. The LAN profile removes
the transport-side stalls on top of that physics; it cannot add airtime.

## Observability

Every transfer logs a summary event (role, mode, streams, bytes, seconds,
Mbps, wire bytes, lost packets/bytes, final CWND, RTT) through `tracing`.
Run the CLI with `--verbose info` (or `debug`) to see it; the GUI mirrors
the same library logs into its own verbosity setting.

## Reproduction

Run `bash scripts/latency-check.sh` on Linux with Rust, Python 3, iproute2,
ping and permission to create network namespaces. All shaping takes place in
an isolated namespace; no host traffic is shaped. The example transfers 8 MiB
over IPv6 QUIC and verifies SHA-256, while ping shares a simulated 10 Mbps
link with 10 ms delay in each direction and a 1,000-packet queue.

Observed in this WSL environment on 2026-10-24:

| Mode | Payload throughput | Mean ping | 95th percentile | Maximum |
|---|---:|---:|---:|---:|
| Unlimited | 7.853 Mbps | 1191.9 ms | 2514.0 ms | 2520.0 ms |
| 7 Mbps payload cap | 6.364 Mbps | 33.6 ms | 44.9 ms | 46.5 ms |

That table predates the WireGuard removal; `--limit-mbps` and the WireGuard
transport no longer exist. Re-run the script after transport changes and
refresh this table rather than trusting the stale numbers. The simulated
link carries data and acknowledgements in both directions; actual gains
depend on uplink capacity, competing traffic and queue management. Router
SQM/AQM can additionally help all applications; Hermod does not change
router settings.

## Protocol settings

QUIC is the only data-plane transport. `--congestion auto` selects BBR;
CUBIC and NewReno remain available for comparison. `--mtu` caps the QUIC
UDP payload (auto = 1452). Selecting an unsupported algorithm fails
explicitly. Neither MTU nor congestion control reserves bandwidth: there
is no rate limiter, so a transfer shares the link with everything else.

Desktop and Android share settings and take a snapshot for each transfer.
Changing settings while transferring affects the next job. Document sources
of unknown length stage to private storage and then send from that file.

## IPv6 and WSL

IPv4 LAN direct transfer is supported; pairing still uses the selected
signaling server. No offline discovery, Internet IPv4 NAT traversal or
payload relay is included. For Internet transfers, both devices need
routed IPv6 and UDP firewall permission. Binding is automatic on
`[::]:0`; the removed `--listen` flag is not a connectivity fix.
Link-local-only and fabricated IPv6 addresses cannot reach remote peers.

On supported Windows 11 and WSL, use the Windows profile `.wslconfig`:

```ini
[wsl2]
networkingMode=mirrored
dnsTunneling=true
```

Run `wsl --shutdown` from Windows after saving. Verify Windows has IPv6
first, then check `ip -6 addr`, `ip -6 route` and the signaling host's
DNS/port from WSL. Mirrored networking still obeys Windows/Hyper-V and
router firewall rules. See Microsoft's
[WSL networking guide](https://learn.microsoft.com/windows/wsl/networking).
Do not disable all firewalls or assign arbitrary public addresses as a
workaround.