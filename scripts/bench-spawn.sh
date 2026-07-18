#!/usr/bin/env bash
# Spawn-latency regression gate.
#
# The product invariant: `hwatu <url>` maps a window as fast as an
# optimized terminal spawns. Any change to engine knobs, prewarm, or the
# IPC path must hold this number. Run before/after such changes.
#
# Usage: scripts/bench-spawn.sh [iterations]
#   HWATU_BENCH_MAX_MS=60   fail if median exceeds this (default 60)
#   HWATU_BENCH_URL=...     page to open (default about:blank)
set -euo pipefail

iters="${1:-10}"
max_ms="${HWATU_BENCH_MAX_MS:-60}"
url="${HWATU_BENCH_URL:-about:blank}"

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$root/target/release"

if [[ ! -x "$bin/hwatu" || ! -x "$bin/hwatud" ]]; then
    echo "bench: building release binaries..." >&2
    cargo build --release --manifest-path "$root/Cargo.toml" >&2
fi

# Fresh daemon so every run measures the same thing (warm pool, no
# leftover windows). NON_UNIQUE means stray daemons don't interfere,
# but kill ours on exit.
"$bin/hwatu" quit >/dev/null 2>&1 || true
sleep 0.3

cleanup() { "$bin/hwatu" quit >/dev/null 2>&1 || true; }
trap cleanup EXIT

# First open spawns the daemon and pays one-time engine/GPU init;
# it is warmup, not a sample.
warmup=$("$bin/hwatu" "$url")
wid=$(awk '{print $2}' <<<"$warmup")
"$bin/hwatu" close "$wid" >/dev/null
echo "warmup: $warmup"

samples=()
for _ in $(seq "$iters"); do
    sleep 0.2 # let the prewarm pool refill on the idle path
    out=$("$bin/hwatu" "$url")
    wid=$(awk '{print $2}' <<<"$out")
    ms=$(grep -o '([0-9]* ms)' <<<"$out" | grep -o '[0-9]*')
    samples+=("$ms")
    "$bin/hwatu" close "$wid" >/dev/null
done

sorted=($(printf '%s\n' "${samples[@]}" | sort -n))
n=${#sorted[@]}
median=${sorted[$((n / 2))]}
p90=${sorted[$((n * 9 / 10 < n ? n * 9 / 10 : n - 1))]}

echo "spawn latency over $n runs (ms): ${sorted[*]}"
echo "min=${sorted[0]} median=$median p90=$p90 max=${sorted[$((n - 1))]}"

if ((median > max_ms)); then
    echo "FAIL: median ${median}ms > ${max_ms}ms budget" >&2
    exit 1
fi
echo "OK: median ${median}ms <= ${max_ms}ms budget"
