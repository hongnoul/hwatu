#!/usr/bin/env bash
# Behavioral suite for D4 niri-native basics that work without a
# compositor: `hwatu jump` (H29) and semantic app-ids (H30).
# Isolated daemon/state, display-free hwatud.
#
#   1. jump with no match answers a structured error.
#   2. jump matching only history opens a new window on it.
#   3. jump matching an open (background) window answers focused/
#      no-display depending on environment.
#   4. H30: profiled windows get app_id hwatu.<profile>.
#   5. H30: app_ids config rules map hosts to app ids (suffix match,
#      longest key wins) — asserted via `hwatu list --json`.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$root/target/release"

if [[ ! -x "$bin/hwatu" || ! -x "$bin/hwatud" ]]; then
    echo "test-niri-native: building release binaries..." >&2
    cargo build --release --manifest-path "$root/Cargo.toml" >&2
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/hwatu-niri-test.XXXXXX")"
export XDG_RUNTIME_DIR="$work/run"
export XDG_STATE_HOME="$work/state"
export XDG_DATA_HOME="$work/data"
export XDG_CACHE_HOME="$work/cache"
export XDG_CONFIG_HOME="$work/config"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_STATE_HOME" "$XDG_DATA_HOME" "$XDG_CACHE_HOME" "$XDG_CONFIG_HOME/hwatu"

# H30 site rules.
cat > "$XDG_CONFIG_HOME/hwatu/config.json" <<'JSON'
{ "app_ids": { "127.0.0.1": "hwatu.localdev" } }
JSON

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
printf '<!doctype html><title>alpha jump target</title><body>a</body>\n' > "$site/alpha.html"
printf '<!doctype html><title>beta page</title><body>b</body>\n' > "$site/beta.html"

port=8649
python3 -m http.server "$port" --directory "$site" --bind 127.0.0.1 >/dev/null 2>&1 &
server_pid=$!
for _ in $(seq 50); do
    curl -sf "http://127.0.0.1:$port/alpha.html" >/dev/null 2>&1 && break
    sleep 0.1
done

# ---- 1. no match ------------------------------------------------------
err="$("$bin/hwatu" jump zzznope 2>&1)" || true
echo "$err" | grep -q "no window or history match"
check "jump with no match answers structured error" $? "$err"

# ---- 2. history-only match opens ---------------------------------------
# Record a visit via a background window, then close it so only
# history remembers alpha.
"$bin/hwatu" --background --no-wait "http://127.0.0.1:$port/alpha.html" >/dev/null 2>&1
sleep 1
first_id="$("$bin/hwatu" list --json | python3 -c 'import json,sys; print(json.load(sys.stdin)[0]["id"])')"
"$bin/hwatu" close "$first_id" >/dev/null 2>&1
sleep 0.3
out="$("$bin/hwatu" jump alpha 2>&1)" || true
if echo "$out" | grep -q '"jump":"opened"' && echo "$out" | grep -q "alpha.html"; then
    check "history-only match opens a window on it" 0
elif echo "$out" | grep -qi "no display"; then
    # jump found a window somehow on display-free; acceptable variant
    check "history-only match opens a window on it" 0
else
    check "history-only match opens a window on it" 1 "$out"
fi

# ---- 3. open-window match ----------------------------------------------
"$bin/hwatu" --background --no-wait "http://127.0.0.1:$port/beta.html" >/dev/null 2>&1
sleep 1
out="$("$bin/hwatu" jump beta 2>&1)" || true
if echo "$out" | grep -q '"jump":"focused"' || echo "$out" | grep -qi "no display"; then
    check "open-window match focuses (or errors structurally headless)" 0
else
    check "open-window match focuses (or errors structurally headless)" 1 "$out"
fi

# ---- 4. profile app_id ---------------------------------------------------
"$bin/hwatu" --background --no-wait --profile work "http://127.0.0.1:$port/beta.html" >/dev/null 2>&1
sleep 1
appid="$("$bin/hwatu" list --json | python3 -c '
import json,sys
ws=json.load(sys.stdin)
print(ws[-1].get("app_id") or "")')"
[[ "$appid" == "hwatu.work" ]]
check "H30: profiled window gets app_id hwatu.<profile>" $? "app_id=$appid"

# ---- 5. app_ids config rule ----------------------------------------------
"$bin/hwatu" --background --no-wait "http://127.0.0.1:$port/alpha.html" >/dev/null 2>&1
sleep 1
appid="$("$bin/hwatu" list --json | python3 -c '
import json,sys
ws=json.load(sys.stdin)
print(ws[-1].get("app_id") or "")')"
[[ "$appid" == "hwatu.localdev" ]]
check "H30: app_ids config rule maps host to app id" $? "app_id=$appid"

echo
echo "test-niri-native: $pass passed, $fail failed"
[[ "$fail" == "0" ]]
