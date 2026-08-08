#!/usr/bin/env bash
# Behavioral suite for link hints (roadmap H10). Runs against a live
# daemon on an ISOLATED socket/state dir, driving the page-side hint
# machinery directly via eval (the keybind path dispatches the same
# __hwatuHints.start call).
#
#   1. Hint mode labels visible interactables (anchor + button), and
#      skips hidden/off-screen ones.
#   2. Typing a label's key activates the target (button click fires).
#   3. Escape dismisses the overlay without activating anything.
#   4. follow on a link navigates.
#   5. A page with no interactables reports "no hints", no overlay.
#
# Usage: scripts/test-hints.sh
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$root/target/release"

if [[ ! -x "$bin/hwatu" || ! -x "$bin/hwatud" ]]; then
    echo "test-hints: building release binaries..." >&2
    cargo build --release --manifest-path "$root/Cargo.toml" >&2
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/hwatu-hints-test.XXXXXX")"
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
eval_js() { # eval_js <id> <js>
    "$bin/hwatu" eval --id "$1" "$2" 2>&1
}

site="$work/site"
mkdir -p "$site"
cat > "$site/hints.html" <<'HTML'
<!doctype html><title>hints fixture</title><body>
<a id="lnk" href="/target.html">a visible link</a>
<button id="btn" onclick="window.__clicked=1">a button</button>
<a id="hidden" href="/nope" style="display:none">hidden</a>
<a id="far" href="/nope" style="position:absolute;top:9000px">off-screen</a>
</body>
HTML
cat > "$site/target.html" <<'HTML'
<!doctype html><title>target page</title><body>arrived</body>
HTML
cat > "$site/empty.html" <<'HTML'
<!doctype html><title>empty</title><body><p>nothing to click</p></body>
HTML

port=8644
python3 -m http.server "$port" --directory "$site" --bind 127.0.0.1 >/dev/null 2>&1 &
server_pid=$!
for _ in $(seq 50); do
    curl -sf "http://127.0.0.1:$port/hints.html" >/dev/null 2>&1 && break
    sleep 0.1
done

# Open one window and drive it via eval (keeps the daemon warm across checks).
out="$("$bin/hwatu" check "http://127.0.0.1:$port/hints.html" --until dom --keep --eval "1" 2>&1)"
id="$(printf '%s' "$out" | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])' 2>/dev/null || echo "")"
if [[ -z "$id" ]]; then
    echo "FAIL  could not open fixture window: $out"
    exit 1
fi

# ---- 1. labels visible interactables only ---------------------------
started="$(eval_js "$id" "return __hwatuHints.start('follow')")"
count_tags="$(eval_js "$id" "return document.querySelectorAll('#__hwatu_hints__ span').length")"
if [[ "$started" == '"2 hints"' && "$count_tags" == "2" ]]; then
    check "labels visible interactables (2: link+button; hidden/off-screen skipped)" 0
else
    check "labels visible interactables (2: link+button; hidden/off-screen skipped)" 1 "start=$started tags=$count_tags"
fi

# ---- 3. Escape dismisses --------------------------------------------
eval_js "$id" "window.dispatchEvent(new KeyboardEvent('keydown', {key: 'Escape'}))" >/dev/null
overlay="$(eval_js "$id" "return !!document.getElementById('__hwatu_hints__')")"
active="$(eval_js "$id" "return __hwatuHints.active()")"
if [[ "$overlay" == "false" && "$active" == "false" ]]; then
    check "Escape dismisses overlay" 0
else
    check "Escape dismisses overlay" 1 "overlay=$overlay active=$active"
fi

# ---- 2. typing a label activates (button) ---------------------------
eval_js "$id" "__hwatuHints.start('follow')" >/dev/null
btn_label="$(eval_js "$id" "
    const tags = [...document.querySelectorAll('#__hwatu_hints__ span')];
    const btnRect = document.getElementById('btn').getBoundingClientRect();
    const tag = tags.reduce((best, t) =>
        Math.abs(parseFloat(t.style.left) - btnRect.left) <
        Math.abs(parseFloat(best.style.left) - btnRect.left) ? t : best);
    return tag.textContent")"
label="$(printf '%s' "$btn_label" | tr -d '"')"
eval_js "$id" "window.dispatchEvent(new KeyboardEvent('keydown', {key: '$label'}))" >/dev/null
clicked="$(eval_js "$id" "return window.__clicked === 1")"
if [[ "$clicked" == "true" ]]; then
    check "typing a hint label activates the target" 0
else
    check "typing a hint label activates the target" 1 "label=$label clicked=$clicked"
fi

# ---- 4. follow on a link navigates ----------------------------------
eval_js "$id" "__hwatuHints.start('follow')" >/dev/null
lnk_label="$(eval_js "$id" "
    const tags = [...document.querySelectorAll('#__hwatu_hints__ span')];
    const r = document.getElementById('lnk').getBoundingClientRect();
    const tag = tags.reduce((best, t) =>
        Math.abs(parseFloat(t.style.left) - r.left) <
        Math.abs(parseFloat(best.style.left) - r.left) ? t : best);
    return tag.textContent")"
label="$(printf '%s' "$lnk_label" | tr -d '"')"
eval_js "$id" "window.dispatchEvent(new KeyboardEvent('keydown', {key: '$label'}))" >/dev/null
"$bin/hwatu" wait-load --id "$id" --until dom >/dev/null 2>&1 || true
where="$(eval_js "$id" "return location.pathname")"
if [[ "$where" == '"/target.html"' ]]; then
    check "hint-follow navigates the link" 0
else
    check "hint-follow navigates the link" 1 "location=$where"
fi

# ---- 5. empty page fails open ---------------------------------------
"$bin/hwatu" goto --id "$id" --until dom "http://127.0.0.1:$port/empty.html" >/dev/null
res="$(eval_js "$id" "return __hwatuHints.start('follow')")"
overlay="$(eval_js "$id" "return !!document.getElementById('__hwatu_hints__')")"
if [[ "$res" == '"no hints"' && "$overlay" == "false" ]]; then
    check "page without interactables fails open" 0
else
    check "page without interactables fails open" 1 "res=$res overlay=$overlay"
fi

echo
echo "test-hints: $pass passed, $fail failed"
[[ "$fail" == "0" ]]
