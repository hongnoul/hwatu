#!/usr/bin/env bash
# Behavioral regression: a page-local Tab press in an unmapped headless WebView
# must establish real WebKit focus, not merely change document.activeElement.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$root/target/debug"
if [[ ! -x "$bin/hwatu" || ! -x "$bin/hwatud" ]]; then
    echo "test-headless-focus: building debug binaries..." >&2
    cargo build --manifest-path "$root/Cargo.toml" -p hwatu -p hwatud >&2
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/hwatu-headless-focus.XXXXXX")"
daemon_pid=""
server_pid=""
cleanup() {
    set +e
    "$bin/hwatu" quit >/dev/null 2>&1 || true
    if [[ -n "$daemon_pid" ]]; then
        for _ in $(seq 1 100); do
            kill -0 "$daemon_pid" 2>/dev/null || break
            sleep 0.02
        done
        kill "$daemon_pid" 2>/dev/null || true
        wait "$daemon_pid" 2>/dev/null || true
    fi
    if [[ -n "$server_pid" ]]; then
        kill "$server_pid" 2>/dev/null || true
        wait "$server_pid" 2>/dev/null || true
    fi
    rm -rf "$work"
}
trap cleanup EXIT INT TERM

# Keep the compositor's runtime dir, but isolate every Hwatu-owned path.
export HWATU_SOCKET="$work/hwatu.sock"
export XDG_STATE_HOME="$work/state"
export XDG_CONFIG_HOME="$work/config"
export XDG_DATA_HOME="$work/data"
export XDG_CACHE_HOME="$work/cache"
mkdir -p "$XDG_STATE_HOME" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" \
    "$XDG_CACHE_HOME" "$work/site"
cat >"$work/site/index.html" <<'HTML'
<!doctype html>
<meta charset="utf-8">
<title>headless focus fixture</title>
<style>
  button:focus { outline: 8px solid rgb(1, 2, 3); }
  button:focus-visible { background: rgb(4, 5, 6); }
</style>
<button id="first">First</button>
<button id="second">Second</button>
HTML

python3 - "$work/site" "$work/port" <<'PY' &
import http.server
import pathlib
import socketserver
import sys

root, port_file = sys.argv[1:]

class Quiet(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *_):
        pass

handler = lambda *args, **kwargs: Quiet(*args, directory=root, **kwargs)
with socketserver.TCPServer(("127.0.0.1", 0), handler) as server:
    pathlib.Path(port_file).write_text(str(server.server_address[1]))
    server.serve_forever()
PY
server_pid=$!
for _ in $(seq 1 100); do
    [[ -s "$work/port" ]] && break
    sleep 0.05
done
[[ -s "$work/port" ]] || { echo "fixture server failed" >&2; exit 1; }
url="http://127.0.0.1:$(cat "$work/port")/index.html"

"$bin/hwatud" >"$work/hwatud.log" 2>&1 &
daemon_pid=$!
for _ in $(seq 1 100); do
    [[ -S "$HWATU_SOCKET" ]] && break
    sleep 0.05
done
if [[ ! -S "$HWATU_SOCKET" ]]; then
    cat "$work/hwatud.log" >&2
    exit 1
fi

"$bin/hwatu" --headless "$url" >/dev/null
"$bin/hwatu" wait-load --until dom >/dev/null
id="$("$bin/hwatu" list --json | python3 -c \
    'import json,sys; d=json.load(sys.stdin); assert len(d)==1,d; print(d[0]["id"])')"

press_json="$("$bin/hwatu" press --id "$id" Tab)"
python3 -c '
import json, sys
reply = json.load(sys.stdin)
assert reply["focused"]["id"] == "first", reply
assert reply["wrapped"] is False, reply
' <<<"$press_json"

focus_json="$("$bin/hwatu" eval --id "$id" '(() => {
  const el = document.querySelector("#first");
  const style = getComputedStyle(el);
  return {
    activeId: document.activeElement && document.activeElement.id,
    focus: el.matches(":focus"),
    focusVisible: el.matches(":focus-visible"),
    outline: style.outline,
    background: style.backgroundColor
  };
})()')"
python3 -c '
import json, sys
state = json.load(sys.stdin)
assert state["activeId"] == "first", state
assert state["focus"] is True, state
assert state["focusVisible"] is True, state
assert state["outline"] == "rgb(1, 2, 3) solid 8px" or state["outline"] == "8px solid rgb(1, 2, 3)", state
assert state["background"] == "rgb(4, 5, 6)", state
' <<<"$focus_json"

# Keep traversal semantics intact after native page focus is established.
second_json="$("$bin/hwatu" press --id "$id" Tab)"
python3 -c '
import json, sys
reply = json.load(sys.stdin)
assert reply["focused"]["id"] == "second", reply
assert reply["wrapped"] is False, reply
' <<<"$second_json"
wrap_json="$("$bin/hwatu" press --id "$id" Tab)"
python3 -c '
import json, sys
reply = json.load(sys.stdin)
assert reply["focused"]["id"] == "first", reply
assert reply["wrapped"] is True, reply
' <<<"$wrap_json"

echo "test-headless-focus: ok"
