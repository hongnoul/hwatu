#!/usr/bin/env bash
# Deterministic E2E smoke test for Hwatu's real CLI -> IPC -> daemon -> WebKit
# agent loop. It follows test-render.sh conventions but starts hwatud itself in
# private XDG dirs, so it cannot attach to or alter the user's daemon/session.
#
# Coverage: headless open/goto/eval/close, composite check + screenshot,
# render --keep followed by type/click/eval, snapshot-ref click, ordered watch
# lifecycle events, virtual-clock determinism, and viewport resize.
#
# Usage: scripts/test-agent-loop.sh
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$root/target/release"

if [[ ! -x "$bin/hwatu" || ! -x "$bin/hwatud" ]]; then
    echo "test-agent-loop: building release binaries..." >&2
    cargo build --release --manifest-path "$root/Cargo.toml" >&2
fi

# shellcheck source=lib/test-harness.sh
source "$root/scripts/lib/test-harness.sh"
hw_private_env "hwatu-agent-loop-test"
hw_set_quit_cmd "$bin/hwatu"
hw_cleanup_hook() {
    # watch_pid is reaped mid-script; kill any leftover first.
    [[ -n "${watch_pid:-}" ]] && kill "$watch_pid" 2>/dev/null || true
}
hw_cleanup_install
# daemon_pid/server_pid/watch_pid stay readable for mid-script use;
# the lib owns their cleanup via hw_track_pid.
daemon_pid=""
server_pid=""
watch_pid=""
hw_counters_init

# Local-only fixtures make navigation deterministic and avoid external DNS/TLS.
fixdir="$work/fixtures"
mkdir -p "$fixdir"
cat > "$fixdir/one.html" <<'HTML'
<!doctype html><meta charset="utf-8"><title>one</title>
<h1 id="page">one</h1>
HTML
cat > "$fixdir/two.html" <<'HTML'
<!doctype html><meta charset="utf-8"><title>two</title>
<h1 id="page">two</h1>
HTML
cat > "$fixdir/interactive.html" <<'HTML'
<!doctype html><meta charset="utf-8"><title>interactive</title>
<input id="name" aria-label="Name">
<button id="apply" onclick="document.body.dataset.result = document.querySelector('#name').value; this.textContent = 'applied'">apply</button>
<button id="ref-button" onclick="this.dataset.clicked = 'yes'">reference target</button>
HTML

# Ask the kernel for a free port and keep the listener in the same Python
# process. The port file is written only after bind succeeds.
python3 - "$fixdir" "$work/port" <<'PY' &
import http.server, pathlib, socketserver, sys
root, port_file = sys.argv[1:]
class Quiet(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *_): pass
handler = lambda *a, **kw: Quiet(*a, directory=root, **kw)
with socketserver.TCPServer(("127.0.0.1", 0), handler) as server:
    pathlib.Path(port_file).write_text(str(server.server_address[1]))
    server.serve_forever()
PY
server_pid=$!
hw_track_pid "$server_pid"
for _ in $(seq 1 100); do [[ -s "$work/port" ]] && break; sleep 0.05; done
if [[ ! -s "$work/port" ]]; then
    echo "FAIL - fixture server did not publish a port" >&2
    exit 1
fi
base="http://127.0.0.1:$(cat "$work/port")"

"$bin/hwatud" >"$work/hwatud.log" 2>&1 &
daemon_pid=$!
hw_track_pid "$daemon_pid"
for _ in $(seq 1 100); do [[ -S "$XDG_RUNTIME_DIR/hwatu.sock" ]] && break; sleep 0.05; done
if [[ ! -S "$XDG_RUNTIME_DIR/hwatu.sock" ]]; then
    echo "FAIL - isolated daemon socket was not created; log follows:" >&2
    cat "$work/hwatud.log" >&2
    exit 1
fi
"$bin/hwatu" ping >/dev/null

# ---- headless open -> goto -> eval -> close ---------------------------
"$bin/hwatu" --headless "$base/one.html" >/dev/null
"$bin/hwatu" wait-load --until dom >/dev/null
id="$("$bin/hwatu" list --json | python3 -c 'import json,sys; d=json.load(sys.stdin); assert len(d)==1,d; print(d[0]["id"])')"
out="$("$bin/hwatu" eval --id "$id" 'document.querySelector("#page").textContent')"
check "headless open reaches fixture DOM" json_assert 'd == "one"' <<<"$out"
"$bin/hwatu" goto --id "$id" --until dom "$base/two.html" >/dev/null
out="$("$bin/hwatu" eval --id "$id" 'document.title + ":" + document.querySelector("#page").textContent')"
check "goto retargets the same headless window" json_assert 'd == "two:two"' <<<"$out"
"$bin/hwatu" close "$id" >/dev/null
check "close removes the headless window" bash -c '[[ $("$0" list --json) == "[]" ]]' "$bin/hwatu"

# ---- composite check with screenshot ---------------------------------
shot="$work/check.png"
out="$("$bin/hwatu" check "$base/one.html" --until dom \
    --eval 'document.querySelector("#page").textContent' --shot="$shot")"
check "composite check returns title and eval" json_assert 'd.get("title") == "one" and d.get("eval") == "one"' <<<"$out"
check "composite check writes a nonempty PNG" bash -c '[[ -s "$1" && $(head -c 8 "$1" | od -An -tx1 | tr -d " \n") == 89504e470d0a1a0a ]]' _ "$shot"

# ---- render --keep + selector and snapshot-ref actions ----------------
out="$("$bin/hwatu" render "$fixdir/interactive.html" --until dom --keep)"
keep_id="$(json_assert 'd.get("rendered") is True and isinstance(d.get("id"), int)' <<<"$out" >/dev/null; python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])' <<<"$out")"
"$bin/hwatu" type --id "$keep_id" '#name' 'Ada' >/dev/null
"$bin/hwatu" click --id "$keep_id" '#apply' >/dev/null
out="$("$bin/hwatu" eval --id "$keep_id" 'document.body.dataset.result + ":" + document.querySelector("#apply").textContent')"
check "render --keep supports type/click DOM mutation" json_assert 'd == "Ada:applied"' <<<"$out"

snap="$("$bin/hwatu" snapshot --id "$keep_id")"
ref="$(python3 -c 'import json,sys; d=json.load(sys.stdin); xs=d.get("interactables", d.get("refs", [])); x=next(x for x in xs if "reference target" in str(x)); print(x.get("ref", x.get("index")))' <<<"$snap")"
"$bin/hwatu" click --id "$keep_id" --ref "$ref" >/dev/null
out="$("$bin/hwatu" eval --id "$keep_id" 'document.querySelector("#ref-button").dataset.clicked')"
check "snapshot ref resolves and dispatches a real click" json_assert 'd == "yes"' <<<"$out"

# ---- ordered event stream --------------------------------------------
"$bin/hwatu" watch --id "$keep_id" --kinds load >"$work/events.jsonl" &
watch_pid=$!
hw_track_pid "$watch_pid"
sleep 0.2
"$bin/hwatu" goto --id "$keep_id" --until dom "$base/two.html" >/dev/null
sleep 0.3
kill "$watch_pid" 2>/dev/null || true
wait "$watch_pid" 2>/dev/null || true
watch_pid=""
check "watch emits monotonic started -> committed -> finished lifecycle" \
    python3 - "$work/events.jsonl" <<'PY'
import json, sys
events = [json.loads(line) for line in open(sys.argv[1]) if line.strip()]
seq = [e["seq"] for e in events]
states = [e.get("data", {}).get("state") for e in events if e.get("event") == "load"]
assert seq and seq == sorted(seq) and len(seq) == len(set(seq)), (seq, events)
pos = [states.index(s) for s in ("started", "committed", "finished")]
assert pos == sorted(pos), states
PY

# ---- deterministic virtual clock -------------------------------------
"$bin/hwatu" clock --id "$keep_id" pause >/dev/null
"$bin/hwatu" clock --id "$keep_id" set 1000 >/dev/null
before="$("$bin/hwatu" eval --id "$keep_id" 'performance.now()')"
"$bin/hwatu" clock --id "$keep_id" step 250 >/dev/null
after="$("$bin/hwatu" eval --id "$keep_id" 'performance.now()')"
check "clock pause/set/step advances exactly 250 ms" python3 - "$before" "$after" <<'PY'
import json, sys
a = float(json.loads(sys.argv[1]))
b = float(json.loads(sys.argv[2]))
assert abs((b-a)-250) < 0.001, (a,b)
PY

# ---- viewport resize --------------------------------------------------
"$bin/hwatu" resize --id "$keep_id" 640x480 >/dev/null
out="$("$bin/hwatu" eval --id "$keep_id" '({width: innerWidth, height: innerHeight})')"
check "resize updates DOM viewport to 640x480" json_assert 'd == {"width":640,"height":480}' <<<"$out"
"$bin/hwatu" close "$keep_id" >/dev/null

echo
echo "test-agent-loop: $pass passed, $fail failed"
if [[ "$fail" -ne 0 ]]; then
    echo "test-agent-loop: artifacts were in $work (removed by cleanup); rerun with bash -x for command traces" >&2
fi
[[ "$fail" -eq 0 ]]
