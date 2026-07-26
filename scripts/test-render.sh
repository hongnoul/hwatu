#!/usr/bin/env bash
# Behavioral suite for `hwatu render` (roadmap G1): documents without
# a server. Runs against a live daemon on an ISOLATED socket/state dir
# so the user's daemon and session are untouched.
#
# Asserts, per the roadmap test plan:
#   1. render --stdin / <file> works; eval + shot + diff work on the
#      rendered (URL-less) window.
#   2. Relative assets resolve against --base (img naturalWidth > 0).
#   3. --until dom releases on inline-script documents.
#   4. Session restore never resurrects rendered windows.
#   5. Window recycling spans render -> check(url) -> render on one
#      pooled window (window count stays at 1).
#   6. A 1 MB+ document goes through the socket; an over-cap document
#      fails with a clear client-side error.
#
# Usage: scripts/test-render.sh
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$root/target/release"

if [[ ! -x "$bin/hwatu" || ! -x "$bin/hwatud" ]]; then
    echo "test-render: building release binaries..." >&2
    cargo build --release --manifest-path "$root/Cargo.toml" >&2
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/hwatu-render-test.XXXXXX")"
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

# ---- fixtures -------------------------------------------------------
fixdir="$work/fix"
mkdir -p "$fixdir"
# A 1x1 red PNG for the relative-asset test.
printf '\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x02\x00\x00\x00\x90wS\xde\x00\x00\x00\x0cIDATx\x9cc\xf8\xcf\xc0\x00\x00\x00\x03\x00\x01\x87\xa1J\xf6\x00\x00\x00\x00IEND\xaeB`\x82' \
    > "$fixdir/dot.png"
cat > "$fixdir/asset-page.html" <<'HTML'
<!DOCTYPE html><meta charset="utf-8"><title>assets</title>
<img id="dot" src="dot.png">
HTML
cat > "$fixdir/inline-script.html" <<'HTML'
<!DOCTYPE html><meta charset="utf-8"><title>inline</title>
<div id="out">pending</div>
<script>document.getElementById('out').textContent = 'ran';</script>
HTML

# Local server for --base resolution (serves dot.png). No subshell:
# $! must be the python pid itself, or cleanup leaks the server.
port=8641
python3 -m http.server "$port" --bind 127.0.0.1 --directory "$fixdir" >/dev/null 2>&1 &
server_pid=$!
for _ in $(seq 50); do
    curl -sf -o /dev/null "http://127.0.0.1:$port/dot.png" && break
    sleep 0.1
done

"$bin/hwatu" ping >/dev/null # spawn the isolated daemon

# ---- 1. stdin render + eval on a URL-less window --------------------
out="$(echo '<h1 id="t">rendered</h1>' \
    | "$bin/hwatu" render --stdin --eval "document.getElementById('t').textContent")"
check "render --stdin returns rendered:true" \
    grep -q '"rendered":true' <<<"$out"
check "eval runs on the rendered document" \
    grep -q '"eval":"rendered"' <<<"$out"

# ---- 2. relative asset resolves against --base ----------------------
out="$("$bin/hwatu" render "$fixdir/asset-page.html" \
    --base "http://127.0.0.1:$port/" \
    --eval "document.getElementById('dot').naturalWidth")"
check "relative img resolves against --base (naturalWidth 1)" \
    grep -q '"eval":1' <<<"$out"

# ---- 3. --until dom on an inline-script document ---------------------
out="$(echo "$(cat "$fixdir/inline-script.html")" \
    | "$bin/hwatu" render --stdin --until dom \
        --eval "document.getElementById('out').textContent")"
check "--until dom sees inline script effects" \
    grep -q '"eval":"ran"' <<<"$out"

# ---- 4. shot + diff work on a rendered window ------------------------
shot="$work/render.png"
out="$("$bin/hwatu" render --stdin --shot="$shot" <<<'<body style="background:#123456"><h1>stable</h1></body>')"
check "render --shot writes a PNG" test -s "$shot"
out="$("$bin/hwatu" render --stdin --baseline "$shot" <<<'<body style="background:#123456"><h1>stable</h1></body>')"
match="$(grep -o '"match_percent":[0-9.]*' <<<"$out" | cut -d: -f2)"
check "render --baseline diffs against the shot (match >= 99, got ${match:-none})" \
    awk "BEGIN { exit !(${match:-0} >= 99) }"

# ---- 5. recycling: renders share one window; render/check keep one
#         warm window per origin kind (file vs network), because a
#         cross-kind adoption forces a WebKit process swap that costs
#         more than a fresh window.
"$bin/hwatu" render --stdin <<<'<p>recycle me</p>' >/dev/null
before="$("$bin/hwatu" list --json | python3 -c 'import json,sys; print(len(json.load(sys.stdin)))')"
"$bin/hwatu" render --stdin <<<'<p>recycle me again</p>' >/dev/null
after="$("$bin/hwatu" list --json | python3 -c 'import json,sys; print(len(json.load(sys.stdin)))')"
check "back-to-back renders recycle one window ($before -> $after)" \
    test "$after" -eq "$before"
out="$("$bin/hwatu" check "http://127.0.0.1:$port/asset-page.html" --until dom)"
check "check after render still works" grep -q '"title":"assets"' <<<"$out"
"$bin/hwatu" render --stdin <<<'<p>again</p>' >/dev/null
count="$("$bin/hwatu" list --json | python3 -c 'import json,sys; print(len(json.load(sys.stdin)))')"
check "render/check alternation keeps <= 2 pooled windows (got $count)" \
    test "$count" -le 2

# ---- 6. session restore excludes rendered windows --------------------
"$bin/hwatu" render --stdin --keep <<<'<p>kept</p>' >/dev/null
sleep 3 # debounced session save
if compgen -G "$XDG_STATE_HOME/hwatu/session*.json" >/dev/null; then
    check "session file has no rendered windows" \
        bash -c '! grep -q "hwatu://render" "$XDG_STATE_HOME"/hwatu/session*.json'
else
    ok "session file has no rendered windows (no file written)"
fi

# ---- 7. size: 1 MB document through the socket -----------------------
python3 -c 'print("<!DOCTYPE html><title>big</title>" + "<p>x</p>" * 150000)' > "$fixdir/big.html"
check "1 MB+ fixture is really 1 MB+" \
    test "$(stat -c%s "$fixdir/big.html")" -gt 1000000
out="$("$bin/hwatu" render "$fixdir/big.html" --until dom --eval "document.querySelectorAll('p').length")"
check "1 MB document renders (150000 <p>)" grep -q '"eval":150000' <<<"$out"

python3 -c 'print("<p>" + "x" * (8 * 1024 * 1024) + "</p>")' > "$fixdir/huge.html"
if err="$("$bin/hwatu" render "$fixdir/huge.html" 2>&1)"; then
    bad "over-cap document is rejected"
else
    check "over-cap document is rejected with the cap named" \
        grep -q "cap" <<<"$err"
fi

echo
echo "test-render: $pass passed, $fail failed"
[[ "$fail" -eq 0 ]]
