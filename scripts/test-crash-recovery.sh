#!/usr/bin/env bash
# Behavioral regression for issue #51: kill the isolated daemon's WebKit web
# process, verify list keeps the URL and termination reason, then explicitly
# navigate to prove the window recovers. No user daemon or external site is used.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$root/target/debug"
if [[ ! -x "$bin/hwatu" || ! -x "$bin/hwatud" ]]; then
    echo "test-crash-recovery: building debug binaries..." >&2
    cargo build --manifest-path "$root/Cargo.toml" >&2
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/hwatu-crash-recovery.XXXXXX")"
daemon_pid=""
server_pid=""
cleanup() {
    set +e
    "$bin/hwatu" quit >/dev/null 2>&1 || true
    if [[ -n "$daemon_pid" ]]; then
        # A recovered WebKit process may still be flushing cache files after
        # the daemon accepts quit. Reap the daemon before removing its XDG
        # roots so a child cannot recreate entries under rm.
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
    # Sandboxed WebKit helpers can outlive their parent for a few scheduler
    # ticks. Retry bounded removal rather than reporting a false test failure
    # or leaking the fixture directory.
    for _ in $(seq 1 100); do
        rm -rf "$work" 2>/dev/null && return
        sleep 0.02
    done
    echo "failed to remove test directory: $work" >&2
    return 1
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
printf '<!doctype html><title>crash sentinel</title><h1>alive</h1>\n' \
    >"$work/site/index.html"

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

# Select only real WebKitWebProcess leaves descended from this isolated daemon,
# excluding the similarly named processes owned by the user's normal daemon.
mapfile -t web_pids < <(python3 - "$daemon_pid" <<'PY'
import subprocess
import sys

root = int(sys.argv[1])
rows = []
for line in subprocess.check_output(
    ["ps", "-eo", "pid=,ppid=,args="], text=True
).splitlines():
    parts = line.strip().split(None, 2)
    if len(parts) == 3:
        rows.append((int(parts[0]), int(parts[1]), parts[2]))
parent = {pid: ppid for pid, ppid, _ in rows}

def descends_from_root(pid):
    seen = set()
    while pid not in seen and pid in parent:
        if parent[pid] == root:
            return True
        seen.add(pid)
        pid = parent[pid]
    return False

for pid, _, args in rows:
    executable = args.split(None, 1)[0]
    if executable.endswith("/WebKitWebProcess") and descends_from_root(pid):
        print(pid)
PY
)
if ((${#web_pids[@]} == 0)); then
    echo "no isolated WebKitWebProcess found" >&2
    cat "$work/hwatud.log" >&2
    exit 1
fi
kill -KILL "${web_pids[@]}"

crash_json=""
for _ in $(seq 1 100); do
    crash_json="$("$bin/hwatu" list --json)"
    if python3 -c \
        'import json,sys; d=json.load(sys.stdin)[0]; assert d.get("web_process_terminated",{}).get("reason")' \
        <<<"$crash_json" 2>/dev/null; then
        break
    fi
    sleep 0.05
done
python3 -c \
    'import json,sys; want=sys.argv[1]; d=json.load(sys.stdin)[0]; t=d["web_process_terminated"]; assert d["url"]==want,(d,want); assert t["url"]==want,t; assert t["reason"] in {"crashed","oom","terminated"},t' \
    "$url" <<<"$crash_json"

# Recovery is explicit. This deliberately performs a safe GET instead of
# attempting to replay an unknown authentication POST.
"$bin/hwatu" goto --id "$id" --until dom "$url" >/dev/null
recovered="$("$bin/hwatu" list --json)"
python3 -c \
    'import json,sys; d=json.load(sys.stdin)[0]; assert d["url"]==sys.argv[1],d; assert "web_process_terminated" not in d,d; assert d["title"]=="crash sentinel",d' \
    "$url" <<<"$recovered"

echo "PASS crash recovery: retained URL/diagnostic and explicit navigation recovered"
