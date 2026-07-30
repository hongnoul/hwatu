#!/usr/bin/env bash
# Behavioral suite for `hwatu net` (roadmap P2 item 9): structured
# per-window network observation. Runs against a live daemon on an
# ISOLATED socket/state dir so the user's daemon and session are
# untouched.
#
# Asserts, per the roadmap spec:
#   1. Loading a page with subresources produces entries with correct
#      method, status, and type (document/stylesheet/script/image).
#   2. A 404 subresource shows status 404 (success-level transport,
#      error-level HTTP).
#   3. A fetch() POST is captured with its method.
#   4. --clear empties the buffer; the next read is a clean diff.
#   5. The ring buffer caps at 500 entries (long-lived windows can't
#      grow the daemon unbounded).
#   6. Timing fields are present and sane (start_ms/duration_ms).
#
# Usage: scripts/test-net.sh
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$root/target/release"

if [[ ! -x "$bin/hwatu" || ! -x "$bin/hwatud" ]]; then
    echo "test-net: building release binaries..." >&2
    cargo build --release --manifest-path "$root/Cargo.toml" >&2
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/hwatu-net-test.XXXXXX")"
export XDG_RUNTIME_DIR="$work/run"
export XDG_STATE_HOME="$work/state"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_STATE_HOME"

server_pid=""
cleanup() {
    "$bin/hwatu" quit >/dev/null 2>&1 || true
    [[ -n "$server_pid" ]] && kill "$server_pid" 2>/dev/null || true
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

# jq-less JSON probing: filter entries with python3, which every dev
# box running this suite already has (test-render.sh set the precedent).
entries() { # entries <json> <python-expr over `e` (an entry dict)>
    python3 -c '
import json, sys
data = json.loads(sys.argv[1])
expr = sys.argv[2]
print(sum(1 for e in data if eval(expr)))
' "$1" "$2"
}

# ---- fixtures -------------------------------------------------------
fixdir="$work/fix"
mkdir -p "$fixdir"
# A 1x1 red PNG subresource.
printf '\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x02\x00\x00\x00\x90wS\xde\x00\x00\x00\x0cIDATx\x9cc\xf8\xcf\xc0\x00\x00\x00\x03\x00\x01\x87\xa1J\xf6\x00\x00\x00\x00IEND\xaeB`\x82' \
    > "$fixdir/dot.png"
cat > "$fixdir/style.css" <<'CSS'
body { background: #fff; }
CSS
cat > "$fixdir/app.js" <<'JS'
window.__loaded = true;
JS
cat > "$fixdir/page.html" <<'HTML'
<!DOCTYPE html><meta charset="utf-8"><title>net</title>
<link rel="stylesheet" href="style.css">
<script src="app.js"></script>
<img src="dot.png">
<img src="missing.png">
HTML

port=8642
python3 -m http.server "$port" --bind 127.0.0.1 --directory "$fixdir" >/dev/null 2>&1 &
server_pid=$!
for _ in $(seq 50); do
    curl -sf -o /dev/null "http://127.0.0.1:$port/dot.png" && break
    sleep 0.1
done
base="http://127.0.0.1:$port"

"$bin/hwatu" ping >/dev/null # spawn the isolated daemon

# ---- 1. subresource page: methods, statuses, types -------------------
"$bin/hwatu" --headless "$base/page.html" >/dev/null
"$bin/hwatu" wait-load >/dev/null
out="$("$bin/hwatu" net)"
check "document entry: GET page.html status 200 type document" \
    test "$(entries "$out" 'e["method"]=="GET" and e["url"].endswith("/page.html") and e.get("status")==200 and e["type"]=="document"')" -ge 1
check "stylesheet entry: style.css status 200 type stylesheet" \
    test "$(entries "$out" 'e["url"].endswith("/style.css") and e.get("status")==200 and e["type"]=="stylesheet"')" -ge 1
check "script entry: app.js status 200 type script" \
    test "$(entries "$out" 'e["url"].endswith("/app.js") and e.get("status")==200 and e["type"]=="script"')" -ge 1
check "image entry: dot.png status 200 type image" \
    test "$(entries "$out" 'e["url"].endswith("/dot.png") and e.get("status")==200 and e["type"]=="image"')" -ge 1

# ---- 2. 404 subresource shows status 404 -----------------------------
check "404 subresource: missing.png status 404" \
    test "$(entries "$out" 'e["url"].endswith("/missing.png") and e.get("status")==404')" -ge 1

# ---- 3. timing fields present and sane --------------------------------
check "every entry has start_ms and duration_ms >= 0" \
    test "$(entries "$out" 'isinstance(e.get("start_ms"), int) and isinstance(e.get("duration_ms"), int)')" -eq "$(entries "$out" 'True')"

# ---- 4. fetch() POST is captured with its method ----------------------
"$bin/hwatu" eval "await fetch('$base/dot.png', { method: 'POST' }).catch(() => 0); 'sent'" >/dev/null
# http.server answers POST with 501; the method is what we assert.
for _ in $(seq 20); do
    out="$("$bin/hwatu" net)"
    [[ "$(entries "$out" 'e["method"]=="POST"')" -ge 1 ]] && break
    sleep 0.2
done
check "fetch POST captured with method POST" \
    test "$(entries "$out" 'e["method"]=="POST" and e["url"].endswith("/dot.png")')" -ge 1

# ---- 5. --clear empties the buffer ------------------------------------
"$bin/hwatu" net --clear >/dev/null
out="$("$bin/hwatu" net)"
check "--clear empties the buffer" test "$(entries "$out" 'True')" -eq 0

# ---- 6. --limit returns only the tail ---------------------------------
"$bin/hwatu" goto "$base/page.html" >/dev/null
out="$("$bin/hwatu" net --limit 2)"
check "--limit 2 returns exactly 2 entries" test "$(entries "$out" 'True')" -eq 2

# ---- 7. ring buffer caps at 500 ---------------------------------------
"$bin/hwatu" net --clear >/dev/null
"$bin/hwatu" eval --timeout-ms 120000 "
for (let i = 0; i < 520; i++) {
  await fetch('$base/dot.png?i=' + i).catch(() => 0);
}
return 'done'" >/dev/null
# Fetch completions land on the main loop; poll until the tail request
# is buffered (or timeout).
for _ in $(seq 50); do
    out="$("$bin/hwatu" net)"
    [[ "$(entries "$out" '"i=519" in e["url"]')" -ge 1 ]] && break
    sleep 0.2
done
count="$(entries "$out" 'True')"
check "buffer caps at 500 entries (got $count)" test "$count" -eq 500
first="$(entries "$out" '"?i=0" in e["url"]')"
last="$(entries "$out" '"i=519" in e["url"]')"
check "oldest entries dropped (i=0 gone, i=519 present)" \
    test "$first" -eq 0 -a "$last" -ge 1

echo
echo "test-net: $pass passed, $fail failed"
[[ "$fail" -eq 0 ]]
