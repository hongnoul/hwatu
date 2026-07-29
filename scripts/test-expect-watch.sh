#!/usr/bin/env bash
# Behavioral suite for `hwatu expect ... --watch` (roadmap G3).
# Runs against a live daemon on an isolated socket/state dir.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$root/target/release"

if [[ ! -x "$bin/hwatu" || ! -x "$bin/hwatud" ]]; then
    echo "test-expect-watch: building release binaries..." >&2
    cargo build --release --manifest-path "$root/Cargo.toml" >&2
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/hwatu-expect-watch-test.XXXXXX")"
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

pass=0
fail=0
ok() { pass=$((pass + 1)); echo "ok   - $1"; }
bad() { fail=$((fail + 1)); echo "FAIL - $1"; }
check() { local desc="$1"; shift; if "$@"; then ok "$desc"; else bad "$desc"; fi; }

"$bin/hwatud" >/dev/null 2>&1 &
daemon_pid=$!
for _ in $(seq 100); do
    [[ -S "$XDG_RUNTIME_DIR/hwatu.sock" ]] && break
    sleep 0.1
done
"$bin/hwatu" ping >/dev/null

open_fixture() {
    local file="$1"
    local html="$2"
    printf '%s' "$html" > "$file"
    local id
    id="$($bin/hwatu --headless "file://$file" | sed -n 's/^window \([0-9][0-9]*\).*/\1/p')"
    "$bin/hwatu" wait-load --id "$id" --until dom >/dev/null
    printf '%s\n' "$id"
}

wait_events() { # wait_events <log> <count> [timeout_seconds]
    local log="$1" count="$2" timeout="${3:-6}"
    python3 - "$log" "$count" "$timeout" <<'PY'
import pathlib, sys, time
path = pathlib.Path(sys.argv[1])
want = int(sys.argv[2])
deadline = time.time() + float(sys.argv[3])
while time.time() < deadline:
    if path.exists() and sum(1 for l in path.read_text().splitlines() if l.strip()) >= want:
        sys.exit(0)
    time.sleep(0.05)
sys.exit(1)
PY
}

python_check() { python3 - "$@"; }

# ---- initial + flip, under paused virtual clock -------------------------
page1="$work/flip.html"
id1="$(open_fixture "$page1" '<!doctype html><button id="status">loading</button>')"
"$bin/hwatu" clock --id "$id1" pause >/dev/null
"$bin/hwatu" expect --id "$id1" '#status' --text ready --watch > "$work/flip.log" & w1=$!
wait_events "$work/flip.log" 1
"$bin/hwatu" eval --id "$id1" "document.querySelector('#status').textContent = 'ready';" >/dev/null
wait_events "$work/flip.log" 3
kill "$w1" 2>/dev/null || true; wait "$w1" 2>/dev/null || true
python_check "$work/flip.log" <<'PY'
import json, sys
evs=[json.loads(l) for l in open(sys.argv[1]) if l.strip()]
assert evs[0]['event']=='subscribed'
xs=[e for e in evs if e['event']=='expect']
assert xs[0]['data']['phase']=='initial' and xs[0]['data']['ok'] is False, xs
assert any(e['data']['phase']=='flip' and e['data']['ok'] is True for e in xs), xs
PY
check "initial false then flip true, even with virtual clock paused" test $? -eq 0

# ---- DOM replacement -----------------------------------------------------
page2="$work/replace.html"
id2="$(open_fixture "$page2" '<!doctype html><main id="root"><p id="status">old</p></main>')"
"$bin/hwatu" expect --id "$id2" '#status' --text new --watch > "$work/replace.log" & w2=$!
wait_events "$work/replace.log" 1
"$bin/hwatu" eval --id "$id2" "document.body.innerHTML='<main id=\"root\"><p id=\"status\">new</p></main>';" >/dev/null
wait_events "$work/replace.log" 3
kill "$w2" 2>/dev/null || true; wait "$w2" 2>/dev/null || true
python_check "$work/replace.log" <<'PY'
import json, sys
xs=[json.loads(l) for l in open(sys.argv[1]) if '"event":"expect"' in l]
assert xs[0]['data']['ok'] is False, xs
assert any(e['data']['phase']=='flip' and e['data']['ok'] is True for e in xs), xs
PY
check "DOM replacement triggers a flip" test $? -eq 0

# ---- navigation termination, no duplicate flips -------------------------
page3="$work/nav1.html"
page4="$work/nav2.html"
printf '<!doctype html><p>next</p>' > "$page4"
id3="$(open_fixture "$page3" '<!doctype html><a id="go" href="nav2.html">go</a><p id="status">ready</p>')"
"$bin/hwatu" expect --id "$id3" '#status' --text ready --watch > "$work/nav.log" & w3=$!
wait_events "$work/nav.log" 2
"$bin/hwatu" click --id "$id3" '#go' >/dev/null || true
wait "$w3" || true
python_check "$work/nav.log" <<'PY'
import json, sys
xs=[json.loads(l) for l in open(sys.argv[1]) if '"event":"expect"' in l]
ph=[e['data']['phase'] for e in xs]
assert ph.count('navigation') == 1, ph
assert ph[-1] == 'navigation', ph
assert not any(p == 'flip' for p in ph[ph.index('navigation')+1:]), ph
PY
check "navigation emits one terminal event and no later duplicate flips" test $? -eq 0

# ---- repeated navigation/reinstall uniqueness ---------------------------
page5="$work/unique1.html"
page6="$work/unique2.html"
printf '<!doctype html><p id="status">fresh</p>' > "$page6"
id4="$(open_fixture "$page5" '<!doctype html><a id="go" href="unique2.html">go</a><p id="status">fresh</p>')"
"$bin/hwatu" expect --id "$id4" '#status' --text fresh --watch > "$work/unique-a.log" & wa=$!
wait_events "$work/unique-a.log" 2
"$bin/hwatu" click --id "$id4" '#go' >/dev/null || true
wait "$wa" || true
"$bin/hwatu" expect --id "$id4" '#status' --text changed --watch > "$work/unique-b.log" & wb=$!
wait_events "$work/unique-b.log" 1
"$bin/hwatu" eval --id "$id4" "document.querySelector('#status').textContent = 'changed';" >/dev/null
wait_events "$work/unique-b.log" 3
kill "$wb" 2>/dev/null || true; wait "$wb" 2>/dev/null || true
python_check "$work/unique-a.log" "$work/unique-b.log" <<'PY'
import json, sys
for path in sys.argv[1:]:
    xs=[json.loads(l) for l in open(path) if '"event":"expect"' in l]
    seq=[e['data']['expect_seq'] for e in xs]
    assert seq == list(range(1, len(seq)+1)), (path, seq)
assert sum(1 for l in open(sys.argv[1]) if '"phase":"navigation"' in l) == 1
assert sum(1 for l in open(sys.argv[2]) if '"phase":"initial"' in l) == 1
PY
check "repeated navigation/reinstall has unique per-watch event sequence" test $? -eq 0

echo
echo "test-expect-watch: $pass passed, $fail failed"
[[ "$fail" -eq 0 ]]
