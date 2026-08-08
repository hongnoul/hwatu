#!/usr/bin/env bash
# Behavioral suite for the generalized human hand-off + queue
# (platform roadmap items 10-11). Isolated daemon/state; hwatud runs
# display-free here, so `--now` and `take` correctly answer with the
# structured no-display error while queue/list/dedup semantics are
# fully verifiable.
#
#   1. `hwatu handoff <id> --reason X` queues; reply reports position.
#   2. `hwatu handoffs` lists entries with reason + waiting_secs.
#   3. Re-queueing the same window updates, not duplicates.
#   4. `hwatu handoffs <id>` (take) on display-free answers the
#      structured no-display error AND removes the entry (measured
#      wait was logged daemon-side).
#   5. Unknown window ids answer structured errors.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$root/target/release"

if [[ ! -x "$bin/hwatu" || ! -x "$bin/hwatud" ]]; then
    echo "test-handoff: building release binaries..." >&2
    cargo build --release --manifest-path "$root/Cargo.toml" >&2
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/hwatu-handoff-test.XXXXXX")"
export XDG_RUNTIME_DIR="$work/run"
export XDG_STATE_HOME="$work/state"
export XDG_DATA_HOME="$work/data"
export XDG_CONFIG_HOME="$work/config"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_STATE_HOME" "$XDG_DATA_HOME" "$XDG_CONFIG_HOME"

cleanup() {
    "$bin/hwatu" quit >/dev/null 2>&1 || true
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

id="$("$bin/hwatu" check "about:blank" --until committed --keep --eval 1 2>/dev/null |
    python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')"

# ---- 1. queue -------------------------------------------------------
out="$("$bin/hwatu" handoff "$id" --reason "solve the captcha" 2>&1)"
echo "$out" | grep -q '"handoff": *"queued"' || echo "$out" | grep -q '"handoff":"queued"'
check "handoff queues with a reason" $? "$out"

# ---- 2. list --------------------------------------------------------
sleep 1
lst="$("$bin/hwatu" handoffs 2>&1)"
echo "$lst" | python3 -c "
import json,sys
d=json.load(sys.stdin)
e=d['handoffs']
assert len(e) == 1, e
assert e[0]['id'] == $id
assert e[0]['reason'] == 'solve the captcha'
assert e[0]['waiting_secs'] >= 1, e[0]
" && check "handoffs lists reason and measured wait" 0 || check "handoffs lists reason and measured wait" 1 "$lst"

# ---- 3. dedup -------------------------------------------------------
"$bin/hwatu" handoff "$id" --reason "updated reason" >/dev/null 2>&1
lst="$("$bin/hwatu" handoffs 2>&1)"
echo "$lst" | python3 -c "
import json,sys
d=json.load(sys.stdin)
e=d['handoffs']
assert len(e) == 1, e
assert e[0]['reason'] == 'updated reason'
" && check "re-queue updates instead of duplicating" 0 || check "re-queue updates instead of duplicating" 1 "$lst"

# ---- 4. take: presents (display) or structured error (headless) -----
# With a session display the take succeeds and reports measured wait;
# on display-free runners it must answer the structured no-display
# error AND leave the entry queued for a later attempt.
take_out="$("$bin/hwatu" handoffs "$id" 2>&1)" || true
lst="$("$bin/hwatu" handoffs 2>&1)"
count="$(echo "$lst" | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["handoffs"]))')"
if echo "$take_out" | grep -q '"handoff":"taken"'; then
    [[ "$count" == "0" ]]
    check "take presents the window, reports waited_secs, consumes entry" $? "take=$take_out count=$count"
elif echo "$take_out" | grep -qi "no display"; then
    [[ "$count" == "1" ]]
    check "take on display-free errors structurally and leaves entry queued" $? "take=$take_out count=$count"
else
    check "take answered something recognizable" 1 "$take_out"
fi

# ---- 5. structured errors --------------------------------------------
err="$("$bin/hwatu" handoff 99999 --reason "x" 2>&1)" || true
echo "$err" | grep -q "no window 99999"
check "unknown window id answers a structured error" $? "$err"

err2="$("$bin/hwatu" handoffs 99999 2>&1)" || true
echo "$err2" | grep -q "no pending handoff"
check "taking an unqueued window answers a structured error" $? "$err2"

echo
echo "test-handoff: $pass passed, $fail failed"
[[ "$fail" == "0" ]]
