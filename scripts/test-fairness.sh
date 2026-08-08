#!/usr/bin/env bash
# Behavioral suite for client fairness (platform item 6b): window
# quota and request-rate bulkheads with structured errors. Isolated
# daemon with tight limits via env so the test is fast.
#
#   1. Open beyond HWATU_MAX_WINDOWS answers "over quota", names the
#      cap, and does not open a window.
#   2. Closing a window frees quota.
#   3. A request burst beyond HWATU_MAX_RPS answers "over rate";
#      ping stays exempt (health checks always work).
#   4. After backing off, requests are admitted again.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$root/target/release"

if [[ ! -x "$bin/hwatu" || ! -x "$bin/hwatud" ]]; then
    echo "test-fairness: building release binaries..." >&2
    cargo build --release --manifest-path "$root/Cargo.toml" >&2
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/hwatu-fairness-test.XXXXXX")"
export XDG_RUNTIME_DIR="$work/run"
export XDG_STATE_HOME="$work/state"
export XDG_DATA_HOME="$work/data"
export XDG_CONFIG_HOME="$work/config"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_STATE_HOME" "$XDG_DATA_HOME" "$XDG_CONFIG_HOME"

daemon_pid=""
cleanup() {
    "$bin/hwatu" quit >/dev/null 2>&1 || true
    [[ -n "$daemon_pid" ]] && kill "$daemon_pid" 2>/dev/null || true
    rm -rf "$work"
}
trap cleanup EXIT

pass=0
fail=0
check() {
    local name="$1" ok="$2" detail="${3:-}"
    if [[ "$ok" == "0" ]]; then
        echo "ok    $name"
        pass=$((pass + 1))
    else
        echo "FAIL  $name${detail:+: $detail}"
        fail=$((fail + 1))
    fi
}

# Tight limits for the test: the CLI auto-spawns hwatud, which
# inherits these (2-window cap, 30 rps sustained / 60 burst).
export HWATU_MAX_WINDOWS=2
export HWATU_MAX_RPS=30
for _ in $(seq 50); do
    "$bin/hwatu" ping >/dev/null 2>&1 && break
    sleep 0.1
done

# ---- 1. window quota ------------------------------------------------
"$bin/hwatu" --headless --no-wait about:blank >/dev/null 2>&1
"$bin/hwatu" --headless --no-wait about:blank >/dev/null 2>&1
sleep 0.3
third="$("$bin/hwatu" --headless --no-wait about:blank 2>&1)" || true
if echo "$third" | grep -q "over quota" && echo "$third" | grep -q "cap 2"; then
    check "third open refused with structured over-quota error" 0
else
    check "third open refused with structured over-quota error" 1 "$third"
fi
count="$("$bin/hwatu" list --json | python3 -c 'import json,sys; print(len(json.load(sys.stdin)))')"
[[ "$count" == "2" ]]
check "no window leaked past the quota (still 2)" $? "count=$count"

# ---- 2. closing frees quota ------------------------------------------
first_id="$("$bin/hwatu" list --json | python3 -c 'import json,sys; print(json.load(sys.stdin)[0]["id"])')"
"$bin/hwatu" close "$first_id" >/dev/null 2>&1
sleep 0.3
again="$("$bin/hwatu" --headless --no-wait about:blank 2>&1)" || true
if echo "$again" | grep -q "over quota"; then
    check "closing a window frees quota" 1 "$again"
else
    check "closing a window frees quota" 0
fi

# ---- 3. rate bulkhead ------------------------------------------------
# Burst list requests well past the 60-token burst bucket.
hits=0
for _ in $(seq 90); do
    out="$("$bin/hwatu" list 2>&1)" || true
    if echo "$out" | grep -q "over rate"; then
        hits=$((hits + 1))
    fi
done
if [[ "$hits" -gt 0 ]]; then
    check "burst beyond the budget answers over-rate ($hits/90 refused)" 0
else
    check "burst beyond the budget answers over-rate" 1 "no refusals in 90 rapid calls"
fi

# Ping stays exempt even mid-exhaustion.
ping_out="$("$bin/hwatu" ping 2>&1)" || true
if echo "$ping_out" | grep -q "over rate"; then
    check "ping exempt from the rate bulkhead" 1 "$ping_out"
else
    check "ping exempt from the rate bulkhead" 0
fi

# ---- 4. recovery ------------------------------------------------------
sleep 2
rec="$("$bin/hwatu" list 2>&1)" || true
if echo "$rec" | grep -q "over rate"; then
    check "requests admitted again after backoff" 1 "$rec"
else
    check "requests admitted again after backoff" 0
fi

echo
echo "test-fairness: $pass passed, $fail failed"
[[ "$fail" == "0" ]]
