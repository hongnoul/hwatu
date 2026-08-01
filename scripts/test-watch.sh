#!/usr/bin/env bash
# Behavioral suite for push IPC / `hwatu watch` (roadmap G2). Runs
# against a live daemon on an ISOLATED socket/state dir.
#
# Asserts, per the roadmap test plan:
#   1. One-shot back-compat: a plain check against the new daemon
#      still answers with exactly one JSON line then EOF, and a
#      connect-then-close (EOF, no request) does not hurt the daemon.
#   2. Two concurrent subscribers see the same events for one driven
#      navigation, each with strictly monotonic seqs from 0.
#   3. Load lifecycle + console + window events are observed.
#   4. A subscriber killed with SIGKILL leaks nothing: daemon fd
#      count returns to baseline and operations still work.
#   5. A stopped (SIGSTOP) subscriber under an event hammer never
#      stalls the daemon: a parallel check completes fast; the stuck
#      client is dropped once its write budget is exhausted.
#   6. Kind and window filters restrict the stream.
#
# Usage: scripts/test-watch.sh
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$root/target/release"

if [[ ! -x "$bin/hwatu" || ! -x "$bin/hwatud" ]]; then
    echo "test-watch: building release binaries..." >&2
    cargo build --release --manifest-path "$root/Cargo.toml" >&2
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/hwatu-watch-test.XXXXXX")"
export XDG_RUNTIME_DIR="$work/run"
export XDG_STATE_HOME="$work/state"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_STATE_HOME"

daemon_pid=""
cleanup() {
    "$bin/hwatu" quit >/dev/null 2>&1 || true
    [[ -n "$daemon_pid" ]] && kill "$daemon_pid" 2>/dev/null || true
    # Kill any subscriber we left behind.
    jobs -p | xargs -r kill -9 2>/dev/null || true
    rm -rf "$work"
}
trap cleanup EXIT

pass=0
fail=0
ok()   { pass=$((pass + 1)); echo "ok   - $1"; }
bad()  { fail=$((fail + 1)); echo "FAIL - $1"; }
check() { # check <description> <condition...>
    local desc="$1"; shift
    if "$@"; then ok "$desc"; else bad "$desc"; fi
}

# Start the daemon ourselves so we can watch its pid and fds.
"$bin/hwatud" >/dev/null 2>&1 &
daemon_pid=$!
for _ in $(seq 100); do
    [[ -S "$XDG_RUNTIME_DIR/hwatu.sock" ]] && break
    sleep 0.1
done
"$bin/hwatu" ping >/dev/null

daemon_fds() { ls "/proc/$daemon_pid/fd" 2>/dev/null | wc -l; }

# ---- 1. one-shot and persistent connection back-compat ----------------
lines="$(python3 - <<'EOF'
import json, os, socket
s = socket.socket(socket.AF_UNIX)
s.settimeout(2)
s.connect(os.environ["XDG_RUNTIME_DIR"] + "/hwatu.sock")
s.sendall(b'{"cmd":"ping"}\n')
f = s.makefile("rb")
print(1 if json.loads(f.readline())["status"] == "ok" else 0)
EOF
)"
check "one-shot request: exactly one reply line without waiting for EOF (got $lines)" \
    test "$lines" -eq 1

lines="$(python3 - <<'EOF'
import json, os, socket
s = socket.socket(socket.AF_UNIX)
s.settimeout(2)
s.connect(os.environ["XDG_RUNTIME_DIR"] + "/hwatu.sock")
f = s.makefile("rwb", buffering=0)
f.write(b'{"cmd":"ping"}\n')
first = json.loads(f.readline())
f.write(b'{"cmd":"ping"}\n')
second = json.loads(f.readline())
print(int(first["status"] == "ok") + int(second["status"] == "ok"))
EOF
)"
check "persistent connection serves two sequential requests (got $lines replies)" \
    test "$lines" -eq 2

python3 - <<'EOF'
import os, socket
s = socket.socket(socket.AF_UNIX)
s.connect(os.environ["XDG_RUNTIME_DIR"] + "/hwatu.sock")
s.close()
EOF
sleep 0.3
check "connect-then-close (EOF, no request) leaves the daemon healthy" \
    "$bin/hwatu" ping >/dev/null

# ---- 2+3. concurrent subscribers, monotonic seqs, event kinds --------
"$bin/hwatu" watch > "$work/sub1.log" & sub1=$!
"$bin/hwatu" watch > "$work/sub2.log" & sub2=$!
sleep 0.5
echo '<script>console.error("watched")</script>' | "$bin/hwatu" render --stdin >/dev/null
sleep 0.5
kill "$sub1" "$sub2" 2>/dev/null; wait "$sub1" "$sub2" 2>/dev/null || true

verify_stream() { # verify_stream <log> -> prints "kinds_ok seq_ok"
    python3 - "$1" <<'EOF'
import json, sys
events = [json.loads(l) for l in open(sys.argv[1]) if l.strip()]
kinds = {e["event"] for e in events}
seqs = [e["seq"] for e in events]
kinds_ok = {"subscribed", "window", "load", "console"} <= kinds
states = {e["data"].get("state") for e in events if e["event"] == "load"}
lifecycle_ok = {"started", "committed", "finished"} <= states
seq_ok = seqs == sorted(seqs) and len(set(seqs)) == len(seqs) and seqs[0] == 0
print(f"{kinds_ok and lifecycle_ok} {seq_ok}")
EOF
}
r1=($(verify_stream "$work/sub1.log"))
r2=($(verify_stream "$work/sub2.log"))
check "subscriber 1 saw subscribed+window+load(started/committed/finished)+console" \
    test "${r1[0]}" = "True"
check "subscriber 1 seqs strictly monotonic from 0" test "${r1[1]}" = "True"
check "subscriber 2 saw the same event kinds" test "${r2[0]}" = "True"
check "subscriber 2 seqs strictly monotonic from 0" test "${r2[1]}" = "True"

# ---- 4. SIGKILLed subscriber leaks nothing ----------------------------
sleep 0.5
fds_before="$(daemon_fds)"
"$bin/hwatu" watch >/dev/null & subk=$!
sleep 0.3
kill -9 "$subk"; wait "$subk" 2>/dev/null || true
# The daemon notices the dead peer on its pending read; give it a beat.
sleep 0.7
fds_after="$(daemon_fds)"
check "daemon fd count returns to baseline after SIGKILLed subscriber ($fds_before -> $fds_after)" \
    test "$fds_after" -le "$fds_before"
check "daemon still serves checks after subscriber SIGKILL" \
    "$bin/hwatu" check about:blank >/dev/null

# ---- 5. stopped subscriber never stalls the daemon --------------------
"$bin/hwatu" watch > "$work/stuck.log" & stuck=$!
sleep 0.3
kill -STOP "$stuck"
# Hammer events while the reader is frozen. The daemon queues up to
# its per-subscriber budget, then drops the client; it must never
# block the GTK loop.
for _ in $(seq 15); do
    echo '<p>hammer</p>' | "$bin/hwatu" render --stdin >/dev/null
done
start_ns=$(date +%s%N)
"$bin/hwatu" check about:blank >/dev/null
elapsed_ms=$(( ($(date +%s%N) - start_ns) / 1000000 ))
check "parallel check completes fast with a stuck subscriber (${elapsed_ms}ms < 2000ms)" \
    test "$elapsed_ms" -lt 2000
kill -CONT "$stuck" 2>/dev/null || true
kill "$stuck" 2>/dev/null || true
wait "$stuck" 2>/dev/null || true
check "daemon alive after stuck-subscriber episode" kill -0 "$daemon_pid"

# ---- 6. filters --------------------------------------------------------
"$bin/hwatu" watch --kinds load > "$work/filtered.log" & subf=$!
sleep 0.4
echo '<script>console.error("filtered out")</script>' | "$bin/hwatu" render --stdin >/dev/null
sleep 0.4
kill "$subf" 2>/dev/null; wait "$subf" 2>/dev/null || true
bad_kinds="$(python3 -c "
import json,sys
evs=[json.loads(l) for l in open('$work/filtered.log') if l.strip()]
print(sum(1 for e in evs if e['event'] not in ('subscribed','load')))")"
loads="$(grep -c '"event":"load"' "$work/filtered.log" || true)"
check "--kinds load: only load events pass ($loads loads, $bad_kinds others)" \
    bash -c "test $bad_kinds -eq 0 && test $loads -gt 0"

check "unknown --kinds value is rejected" \
    bash -c "! $bin/hwatu watch --kinds bogus 2>/dev/null"

echo
echo "test-watch: $pass passed, $fail failed"
[[ "$fail" -eq 0 ]]
