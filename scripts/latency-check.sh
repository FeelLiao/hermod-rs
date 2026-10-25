#!/usr/bin/env bash
# 所有 tc 规则只存在于临时网络命名空间，不改变宿主机网卡。
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release -p hermod-core --example ipv6_load_probe
unshare --net bash -c '
set -euo pipefail
ip link set lo up
tc qdisc add dev lo root netem delay 10ms rate 10mbit limit 1000
python3 - <<"PY"
import pathlib, re, statistics, subprocess, time
binary = str(pathlib.Path("target/release/examples/ipv6_load_probe").resolve())
for limit in ("0", "7"):
    ping = subprocess.Popen(["ping", "-6", "-i", "0.1", "::1"], stdout=subprocess.PIPE, text=True)
    try:
        probe = subprocess.run([binary, limit], capture_output=True, text=True, timeout=120)
        time.sleep(1)
    finally:
        ping.terminate()
        output = ping.communicate(timeout=5)[0]
    samples = sorted(float(x) for x in re.findall(r"time=([\d.]+) ms", output))
    print(probe.stdout, end="", flush=True)
    if probe.returncode:
        raise RuntimeError(probe.stderr)
    if not samples:
        raise RuntimeError("No ping responses")
    print(f"ping samples={len(samples)} mean={statistics.mean(samples):.1f}ms p95={samples[int(len(samples)*0.95)]:.1f}ms max={max(samples):.1f}ms", flush=True)
    time.sleep(2)
PY
'
