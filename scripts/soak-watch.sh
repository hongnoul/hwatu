#!/usr/bin/env bash
# Push-IPC soak (roadmap G2): a subscriber attached while a check
# loop hammers the daemon. Asserts flat memory (RSS delta < 10%) and
# stable fd count over the run. The roadmap's 1-hour soak is
# `scripts/soak-watch.sh 3600`; CI/dev smoke is the 60 s default.
#
# Usage: scripts/soak-watch.sh [seconds]
set -euo pipefail

secs="${1:-60}"
root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$root/target/release"

if [[ ! -x "$bin/hwatu" || ! -x "$bin/hwatud" ]]; then
    echo "soak-watch: building release binaries..." >&2
    cargo build --release --manifest-path "$root/Cargo.toml" >&2
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/hwatu-watch-soak.XXXXXX")"
export XDG_RUNTIME_DIR="$work/run"
export XDG_STATE_HOME="$work/state"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_STATE_HOME"

daemon_pid=""
cleanup() {
    "$bin/hwatu" quit >/dev/null 2>&1 || true
    [[ -n "$daemon_pid" ]] && kill "$daemon_pid" 2>/dev/null || true
    jobs -p | xargs -r kill -9 2>/dev/null || true
    rm -rf "$work"
}
trap cleanup EXIT

"$bin/hwatud" >/dev/null 2>&1 &
daemon_pid=$!
for _ in $(seq 100); do
    [[ -S "$XDG_RUNTIME_DIR/hwatu.sock" ]] && break
    sleep 0.1
done
"$bin/hwatu" ping >/dev/null

rss_kb() { awk '/VmRSS/ { print $2 }' "/proc/$daemon_pid/status"; }
fds()    { ls "/proc/$daemon_pid/fd" | wc -l; }

"$bin/hwatu" watch > "$work/events.log" & sub=$!

# Warm through WebKit's allocator ramp and GLib's lazy system-monitor
# initialization before taking the baseline. A handful of checks is not
# enough: the daemon opens cgroup/proc monitors several seconds after start,
# which made short soaks report pre-existing startup growth as a G2 leak.
warm_deadline=$(( $(date +%s) + 10 ))
warm_checks=0
while (( $(date +%s) < warm_deadline )); do
    "$bin/hwatu" check about:blank >/dev/null
    warm_checks=$((warm_checks + 1))
done
rss0="$(rss_kb)"; fds0="$(fds)"
echo "soak-watch: warmed with $warm_checks checks; ${secs}s baseline RSS ${rss0} kB, ${fds0} fds"

deadline=$(( $(date +%s) + secs ))
checks=0
while (( $(date +%s) < deadline )); do
    "$bin/hwatu" check about:blank >/dev/null
    checks=$((checks + 1))
    if (( checks % 50 == 0 )); then
        echo "JCODE_PROGRESS {\"message\":\"$checks checks, RSS $(rss_kb) kB, $(fds) fds\"}"
    fi
done

rss1="$(rss_kb)"; fds1="$(fds)"
events="$(wc -l < "$work/events.log")"
kill "$sub" 2>/dev/null; wait "$sub" 2>/dev/null || true

echo "soak-watch: $checks checks, $events events streamed"
echo "soak-watch: RSS ${rss0} -> ${rss1} kB, fds ${fds0} -> ${fds1}"

fail=0
# RSS: allow 10% growth (allocator noise); flag anything beyond.
if (( rss1 > rss0 + rss0 / 10 )); then
    echo "FAIL: RSS grew more than 10%"
    fail=1
fi
if (( fds1 > fds0 + 2 )); then
    echo "FAIL: fd count grew (${fds0} -> ${fds1})"
    fail=1
fi
if ! kill -0 "$daemon_pid" 2>/dev/null; then
    echo "FAIL: daemon died during soak"
    fail=1
fi
if (( events < checks )); then
    # Every check produces at least a few load events; far fewer
    # events than checks means the stream silently died.
    echo "FAIL: subscriber saw only $events events for $checks checks"
    fail=1
fi
[[ "$fail" -eq 0 ]] && echo "OK: memory flat, fds stable, stream lived"
exit "$fail"
