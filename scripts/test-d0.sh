#!/usr/bin/env bash
# Behavioral checks for roadmap D0 (browser.md H1-H8), the automatable
# subset. Runs against a live daemon on an ISOLATED socket/state dir.
#
#   1. Daemon boots with the D0 wiring (uploads, notifications, print,
#      spell check, site store) and passes a render smoke test.
#   2. H8 PDF viewing: an application/pdf main resource renders in the
#      built-in viewer instead of converting into a download.
#   3. H5 per-site zoom: a persisted zoom level in site.json is applied
#      on navigation to that host (detected via CSS viewport shrink).
#   4. H5 store: a permission decision file survives daemon restart
#      (store loads, daemon still healthy).
#
# H1 (file dialog), H2 (live WebRTC call), H4 (desktop notification
# click), H6 (squiggles), H7 (print dialog) need a human/display and
# are covered by unit tests + manual verification notes in the PR.
#
# Usage: scripts/test-d0.sh
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$root/target/release"

if [[ ! -x "$bin/hwatu" || ! -x "$bin/hwatud" ]]; then
    echo "test-d0: building release binaries..." >&2
    cargo build --release --manifest-path "$root/Cargo.toml" >&2
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/hwatu-d0-test.XXXXXX")"
export XDG_RUNTIME_DIR="$work/run"
export XDG_STATE_HOME="$work/state"
export XDG_DATA_HOME="$work/data"
export XDG_CONFIG_HOME="$work/config"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_STATE_HOME" "$XDG_DATA_HOME" "$XDG_CONFIG_HOME"

server_pid=""
cleanup() {
    "$bin/hwatu" quit >/dev/null 2>&1 || true
    [[ -n "$server_pid" ]] && kill "$server_pid" 2>/dev/null || true
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

# ---- fixture server ------------------------------------------------
site="$work/site"
mkdir -p "$site"
# Minimal valid single-page PDF.
printf '%%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]>>endobj\nxref\n0 4\n0000000000 65535 f \n0000000009 00000 n \n0000000052 00000 n \n0000000101 00000 n \ntrailer<</Size 4/Root 1 0 R>>\nstartxref\n164\n%%%%EOF\n' > "$site/doc.pdf"
cat > "$site/page.html" <<'HTML'
<!doctype html><title>d0 fixture</title><body><h1>hello</h1></body>
HTML

port=8641
python3 -m http.server "$port" --directory "$site" --bind 127.0.0.1 >/dev/null 2>&1 &
server_pid=$!
for _ in $(seq 50); do
    curl -sf "http://127.0.0.1:$port/page.html" >/dev/null 2>&1 && break
    sleep 0.1
done

# ---- 3. persisted zoom, written BEFORE daemon start ----------------
mkdir -p "$XDG_DATA_HOME/hwatud"
cat > "$XDG_DATA_HOME/hwatud/site.json" <<'JSON'
{ "permissions": { "example.com:camera": true }, "zoom": { "127.0.0.1": 2.0 } }
JSON

# ---- 1. daemon boots + renders ------------------------------------
out="$("$bin/hwatu" check "http://127.0.0.1:$port/page.html" --until dom \
    --eval "document.querySelector('h1').textContent" 2>&1)" || true
[[ "$out" == *"hello"* ]]
check "daemon boots and renders with D0 wiring" $?

# ---- 2. PDF renders instead of downloading -------------------------
# A download conversion aborts the frame load; the check would fail or
# hang, so a successful eval against the viewer is the assertion.
out="$("$bin/hwatu" check "http://127.0.0.1:$port/doc.pdf" --until committed \
    --eval "'loaded:' + location.pathname" 2>&1)" || true
[[ "$out" == *"loaded:/doc.pdf"* ]]
check "H8: application/pdf renders (no download conversion)" "$?" "$out"

# ---- 3. per-site zoom applied on navigation ------------------------
# site.json zooms host 127.0.0.1 to 2.0; the same server reached as
# `localhost` has no entry. Zoom halves the CSS viewport, so the
# zoomed host must report about half the unzoomed host's innerWidth
# (exact 2:1 modulo rounding), regardless of the machine's viewport.
out_plain="$("$bin/hwatu" check "http://localhost:$port/page.html" --until dom \
    --eval "window.innerWidth" 2>&1)" || true
out_zoom="$("$bin/hwatu" check "http://127.0.0.1:$port/page.html" --until dom \
    --eval "window.innerWidth" 2>&1)" || true
w_plain="$(printf '%s' "$out_plain" | python3 -c 'import json,sys; print(json.load(sys.stdin)["eval"])' 2>/dev/null || echo 0)"
w_zoom="$(printf '%s' "$out_zoom" | python3 -c 'import json,sys; print(json.load(sys.stdin)["eval"])' 2>/dev/null || echo 0)"
if [[ "$w_plain" -gt 0 && "$w_zoom" -gt 0 ]] \
    && (( w_zoom * 2 >= w_plain - 4 && w_zoom * 2 <= w_plain + 4 )); then
    check "H5: persisted per-site zoom applied on load" 0
else
    check "H5: persisted per-site zoom applied on load" 1 "plain=$w_plain zoomed=$w_zoom (want 2:1)"
fi

# ---- 4. store survives restart, daemon healthy ---------------------
"$bin/hwatu" quit >/dev/null 2>&1 || true
sleep 0.5
out="$("$bin/hwatu" check "http://127.0.0.1:$port/page.html" --until dom \
    --eval "1+1" 2>&1)" || true
[[ "$out" == *"2"* ]]
check "H5: daemon restarts cleanly with populated site store" $?
grep -q '"example.com:camera": true' "$XDG_DATA_HOME/hwatud/site.json"
check "H5: permission entries retained on disk" $?

echo
echo "test-d0: $pass passed, $fail failed"
[[ "$fail" == "0" ]]
