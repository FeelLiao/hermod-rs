# Throughput tuning and responsiveness

Hermod transfers payload end-to-end over QUIC (UDP), with BBR by default.
**CLI tuning is implementation in progress, awaiting user verification.** The
settings below describe the confirmed implementation, not measured speed gains.
See the [implementation plan](docs/cli-transport-tuning.md) for decisions and acceptance items.

## Invocation-only tuning

CLI tuning uses explicit arguments for this invocation, otherwise built-in defaults.
It is not persisted and is not read from YAML; existing server/pin configuration
still persists separately. This work does not modify GUI/bridge settings or promise
compatibility with their old `compatible` values. LinkProfile has only Auto/Lan/Wan;
there is no Compatible variant or alias.

All data-plane arguments are errors if explicitly supplied to serve/trust, even
when their value equals a default. They are validated before network access.

| Option | Default | Accepted range |
| --- | --- | --- |
| `--streams` | auto | auto or 1–8 |
| `--chunk` | 1MiB | 256KiB–4MiB |
| `--window` | 64MiB | 8MiB–256MiB per stream |
| `--udp-buffer` | 16MiB | 1MiB–64MiB requested socket buffer |
| `--slice-size` | 4MiB | 4MiB–64MiB target |
| `--prefetch` | 16 | 4–64 chunks |
| `--keepalive` | 10 seconds | 0–60; 0 disables it |
| `--idle-timeout` | 30 seconds | 10–300, strictly greater than keepalive |

Sizes accept integer bytes or an integer followed by case-sensitive KiB/MiB;
`1048576`, `1024KiB` and `1MiB` are equivalent. Decimals, MB and embedded spaces
are rejected. Values are not silently clamped.

`--mtu/-m auto` starts QUIC PMTUD at 1200 bytes and probes to 1452; an explicit
1200–1452 cap is never exceeded. `--congestion/-c` accepts auto/bbr/cubic/newreno
(case-insensitive), with auto selecting BBR. Neither setting reserves bandwidth.
There is no payload rate limiter; router SQM/AQM or external shaping can help
responsiveness under saturation. Hermod does not change router settings.

## Receiver-owned concurrency and link profiles

The receiver chooses the payload mode and opens the data streams. Its explicit
`--streams` overrides automatic profile concurrency, subject to source capability,
file size and available slices. Sender-side tuning is a local resource budget:
`tx --streams 1` cannot force a receiver to use one stream or prevent a legal
receiver from opening up to eight.

- `lan`: target up to eight parallel streams, using a slice plan. Slices are written
  at their file offsets and read back in order for SHA-256 verification.
- `wan`: single-stream chunk pipeline by default.
- `auto`: classify the actual peer address. Private/link-local/loopback IPv4 and
  IPv6 ULA/link-local/loopback are LAN candidates; other addresses are WAN.
  IPv4-mapped IPv6 follows IPv4 classification. If a valid RTT is available,
  LAN multi-stream selection additionally requires RTT ≤25ms. Address type is a
  heuristic, not proof of physical locality.

The LAN/WAN initial RTT estimates are 25/100ms. No CLI initial-rtt, ack-delay or
advanced flag is exposed. Multi-stream requires the sender to advertise a seekable
source and a sufficiently large file; otherwise the receiver uses single-stream.
Chunk size is local I/O granularity, not an on-wire frame-size requirement. The
receiver's slice target determines its plan; peers need not match local budgets.
Payload v3, its mode/plan framing and digest verdict remain unchanged. No HTTP,
WireGuard, legacy payload compatibility or 0-RTT path is added.

## Flow control and memory tradeoffs

The per-stream receive window defaults to 64MiB. Connection receive and send
budgets currently use **window × 8**, independently of local streams preference,
so a legal eight-stream peer does not exhaust an undersized local connection budget.
That is 512MiB by default and at most 2GiB. These are budgets, not an immediate
allocation of that amount; larger budgets nevertheless permit more buffered/in-flight
data and can increase memory pressure. Control-stream allowance is separate from
payload concurrency.

A 64MiB total connection window may constrain aggregate in-flight data on some
paths, but does not prove lost throughput: RTT, available bandwidth, loss, disk and
application scheduling all matter. No before/after performance measurement for this
change is recorded, so neither a speedup nor a guaranteed single-stream baseline
is claimed.

- The chunk × prefetch payload budget of one queue defaults to about 16MiB;
  allowed combinations span about 1–256MiB. Active chunks, concurrent stream
  tasks, QUIC buffers, file handles and OS page cache are additional. This product
  is not a process-memory ceiling.
- Larger chunks reduce some per-block overhead but increase per-task working data;
  deeper prefetch can hide I/O latency while retaining more queued bytes.
- Larger slice targets change task granularity, disk access and read-back hashing;
  they do not require a slice-sized in-memory reassembly allocation.
- UDP receive/send buffers are requests to the OS. Kernel limits may cap or reject
  them; check logs rather than assuming the requested allocation was granted.
- Simultaneously raising windows, chunks, prefetch and concurrency compounds resource
  pressure. More streams cannot create Wi-Fi airtime, and can increase scheduling
  or storage overhead. Benchmark representative devices and paths before tuning.

## Idle timeout and observability

`--keepalive` configures QUIC keepalive. `--idle-timeout` configures both QUIC idle
timeout and the CLI payload no-progress watchdog. Keepalive traffic does not by
itself establish payload progress. Pairing/handshake has its own independent budget;
idle-timeout is not a total connection-setup timeout. serve's `--ttl` is a separate
service/channel lifetime setting.

Transfer summary events include role, mode, streams, bytes, duration, Mbps, wire
bytes, lost packets/bytes, final CWND and RTT through tracing. Use `--verbose info`
for summaries or `--verbose debug` to inspect effective tuning/connection budgets.
Record actual stream count, source type, both peers' flags, throughput, latency,
CPU and peak memory when comparing settings; one throughput number is insufficient.

## Reproduction and evidence status

The manual `scripts/latency-check.sh` probe uses an isolated Linux network namespace
and requires Rust, Python 3, iproute2, ping and namespace permissions. It is not
current acceptance evidence until rerun against this implementation.

The old 2026-10-24 WSL test reported 7.853Mbps / 1191.9ms mean ping without a cap,
and 6.364Mbps / 33.6ms with a 7Mbps cap. That experiment predates removal of
WireGuard and the bandwidth limiter; those modes and flags cannot be reproduced
with the current CLI. These historical figures do not measure payload v3 or this
tuning work and are not a performance claim. Collect a new baseline and tuned run
before publishing conclusions.

## Address families, IPv6 and WSL

Global `-4/--ipv4` and `-6/--ipv6` are mutually exclusive and strictly apply to
all commands: tx/rx UDP binding and candidates, signaling DNS/TCP, trust certificate
fetching, and serve TCP/UDP listeners. Explicit restriction never falls back to the
other family; an empty matching address set is an error. Without restriction,
automatic binding uses available dual-stack/fallback behavior.

serve defaults to dual-stack; -4 binds IPv4 only and -6 binds IPv6-only. Use numeric
`serve -p/--port`, resolved CLI > HERMOD_PORT > 4376. The removed listen-address
flag and HERMOD_LISTEN are not connectivity fixes. UDP reflection follows the
listener's port/family; this is not a payload relay.

IPv4 LAN direct transfer is supported; pairing still requires the selected signaling
server. No offline discovery, Internet IPv4 NAT traversal or payload relay is
included. Internet peer transfers need routed IPv6 and UDP firewall permission.
Link-local-only or fabricated IPv6 addresses cannot reach remote Internet peers.

On supported Windows 11 and WSL, use the Windows profile's `.wslconfig`:

```ini
[wsl2]
networkingMode=mirrored
dnsTunneling=true
```

Run `wsl --shutdown` from Windows after saving. Verify Windows has IPv6 first, then
check `ip -6 addr`, `ip -6 route` and the signaling host's DNS/port from WSL.
Mirrored networking still obeys Windows/Hyper-V and router firewall rules. See
Microsoft's [WSL networking guide](https://learn.microsoft.com/windows/wsl/networking).
Do not disable all firewalls or assign arbitrary public addresses as a workaround.
