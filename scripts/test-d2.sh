#!/usr/bin/env bash
# Behavioral suite for D2 cosmetic filtering (roadmap H14) and forced
# dark mode (H15). Isolated daemon/state.
#
#   1. A user cosmetic rule (##.ad-banner) hides matching elements
#      (computed display:none) while content stays visible.
#   2. A scoped rule (host##selector) applies on that host.
#   3. Dark mode: with "dark_mode": true, prefers-color-scheme
#      resolves dark for pages; per-site opt-out works.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$root/target/release"

if [[ ! -x "$bin/hwatu" || ! -x "$bin/hwatud" ]]; then
    echo "test-d2: building release binaries..." >&2
    cargo build --release --manifest-path "$root/Cargo.toml" >&2
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/hwatu-d2-test.XXXXXX")"
export XDG_RUNTIME_DIR="$work/run"
export XDG_STATE_HOME="$work/state"
export XDG_DATA_HOME="$work/data"
export XDG_CONFIG_HOME="$work/config"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_STATE_HOME" "$XDG_DATA_HOME" "$XDG_CONFIG_HOME/hwatu"

# User cosmetic filters: generic and host-scoped.
cat > "$XDG_CONFIG_HOME/hwatu/filters.txt" <<'FILTERS'
##.ad-banner
127.0.0.1##.host-scoped-ad
FILTERS

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

site="$work/site"
mkdir -p "$site"
cat > "$site/ads.html" <<'HTML'
<!doctype html><title>ads fixture</title><body>
<div class="ad-banner">BUY NOW</div>
<div class="host-scoped-ad">HOST AD</div>
<p id="content">real content</p>
</body>
HTML

port=8646
python3 -m http.server "$port" --directory "$site" --bind 127.0.0.1 >/dev/null 2>&1 &
server_pid=$!
for _ in $(seq 50); do
    curl -sf "http://127.0.0.1:$port/ads.html" >/dev/null 2>&1 && break
    sleep 0.1
done

# Wait for the ruleset to compile (first boot compiles user filters;
# poll adblock status until it reports ready).
"$bin/hwatu" ping >/dev/null 2>&1 || true
for _ in $(seq 100); do
    status="$("$bin/hwatu" adblock status 2>&1)" || true
    [[ "$status" == *"compiling"* ]] || break
    sleep 0.2
done

probe() { # probe -> "<banner>|<hostad>|<content>"
    "$bin/hwatu" check "http://127.0.0.1:$port/ads.html" --until settled --eval "
        const d = (s) => { const e = document.querySelector(s); return e ? getComputedStyle(e).display : 'gone'; };
        return d('.ad-banner') + '|' + d('.host-scoped-ad') + '|' + d('#content')" 2>&1 |
        python3 -c 'import json,sys; print(json.load(sys.stdin)["eval"])' 2>/dev/null || echo parse-error
}

res="$(probe)"
# Content blockers apply at load; allow one retry in case the first
# check raced the ruleset application to the prewarmed view.
if [[ "$res" != "none|none|block" ]]; then
    sleep 1
    res="$(probe)"
fi
IFS='|' read -r banner hostad content <<< "$res"

if [[ "$banner" == "none" && "$content" == "block" ]]; then
    check "H14: generic cosmetic rule hides the ad, content survives" 0
else
    check "H14: generic cosmetic rule hides the ad, content survives" 1 "$res"
fi
if [[ "$hostad" == "none" ]]; then
    check "H14: host-scoped cosmetic rule applies on its host" 0
else
    check "H14: host-scoped cosmetic rule applies on its host" 1 "$res"
fi

# ---- H15: forced dark mode via per-site store -----------------------
"$bin/hwatu" quit >/dev/null 2>&1 || true
sleep 0.3
mkdir -p "$XDG_DATA_HOME/hwatud"
cat > "$XDG_DATA_HOME/hwatud/site.json" <<'JSON'
{ "dark": { "127.0.0.1": true } }
JSON
dark="$("$bin/hwatu" check "http://127.0.0.1:$port/ads.html" --until settled \
    --eval "return !!document.getElementById('__hwatu_dark__')" 2>&1 |
    python3 -c 'import json,sys; print(json.load(sys.stdin)["eval"])' 2>/dev/null || echo parse-error)"
if [[ "$dark" == "True" || "$dark" == "true" ]]; then
    check "H15: per-site dark override injects the darkener on load" 0
else
    check "H15: per-site dark override injects the darkener on load" 1 "dark=$dark"
fi

# ---- H16: clear-site-data ------------------------------------------
# Set a cookie + localStorage, clear for the host, verify both gone.
"$bin/hwatu" check "http://127.0.0.1:$port/ads.html" --until dom --keep \
    --eval "document.cookie='d2test=1; path=/'; localStorage.setItem('d2','x'); return 1" >/dev/null 2>&1
out="$("$bin/hwatu" clear-site-data 127.0.0.1 2>&1)"
if echo "$out" | grep -q '"cleared"'; then
    check "H16: clear-site-data answers with a cleared count" 0
else
    check "H16: clear-site-data answers with a cleared count" 1 "$out"
fi
after="$("$bin/hwatu" check "http://127.0.0.1:$port/ads.html" --until dom \
    --eval "return document.cookie + '|' + (localStorage.getItem('d2') || 'null')" 2>&1 |
    python3 -c 'import json,sys; print(json.load(sys.stdin)["eval"])' 2>/dev/null || echo parse-error)"
if [[ "$after" == "|null" ]]; then
    check "H16: cookies and localStorage actually cleared" 0
else
    check "H16: cookies and localStorage actually cleared" 1 "after=$after"
fi

# ---- H19: restore_session=true restores after clean quit ------------
"$bin/hwatu" quit >/dev/null 2>&1 || true
sleep 0.3
cat > "$XDG_CONFIG_HOME/hwatu/config.json" <<'JSON'
{ "restore_session": true }
JSON
"$bin/hwatu" --background "http://127.0.0.1:$port/ads.html" >/dev/null 2>&1 || true
sleep 1
"$bin/hwatu" quit >/dev/null 2>&1 || true
sleep 0.5
listed="$("$bin/hwatu" list 2>&1)" || true
sleep 1.5
listed="$("$bin/hwatu" list 2>&1)" || true
if echo "$listed" | grep -q "ads.html"; then
    check "H19: restore_session=true restores windows after clean quit" 0
else
    check "H19: restore_session=true restores windows after clean quit" 1 "$listed"
fi

echo
echo "test-d2: $pass passed, $fail failed"
[[ "$fail" == "0" ]]
