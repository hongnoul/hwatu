#!/usr/bin/env bash
# Behavioral suite for global history + URL completion (roadmap H9).
# Runs against a live daemon on an ISOLATED socket/state dir.
#
#   1. Committed navigations in a Normal window are recorded with
#      visit counts (repeat visit increments, not duplicates).
#   2. Headless (agent) checks are NOT recorded.
#   3. `hwatu history <query>` returns frecency-ranked completions,
#      host-prefix match first; titles attach once pages deliver them.
#   4. Internal pages (launcher, about:blank) never appear.
#   5. History survives a daemon restart (SQLite on disk).
#   6. `hwatu history --clear` wipes and reports the count.
#
# Usage: scripts/test-history.sh
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$root/target/release"

if [[ ! -x "$bin/hwatu" || ! -x "$bin/hwatud" ]]; then
    echo "test-history: building release binaries..." >&2
    cargo build --release --manifest-path "$root/Cargo.toml" >&2
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/hwatu-history-test.XXXXXX")"
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

site="$work/site"
mkdir -p "$site"
cat > "$site/alpha.html" <<'HTML'
<!doctype html><title>Alpha Fixture Page</title><body>alpha</body>
HTML
cat > "$site/beta.html" <<'HTML'
<!doctype html><title>Beta Fixture Page</title><body>beta</body>
HTML

port=8642
python3 -m http.server "$port" --directory "$site" --bind 127.0.0.1 >/dev/null 2>&1 &
server_pid=$!
for _ in $(seq 50); do
    curl -sf "http://127.0.0.1:$port/alpha.html" >/dev/null 2>&1 && break
    sleep 0.1
done

alpha="http://127.0.0.1:$port/alpha.html"
beta="http://127.0.0.1:$port/beta.html"

# ---- 1. record + count normal-window navigations -------------------
# Open a background window (recorded like Normal; headless is not) and
# navigate it twice to alpha, once to beta.
win_json="$("$bin/hwatu" --background "$alpha" 2>&1)" || true
sleep 1
win_id="$("$bin/hwatu" list --json | python3 -c 'import json,sys; print(json.load(sys.stdin)[0]["id"])')"
"$bin/hwatu" goto --id "$win_id" --until dom "$beta" >/dev/null
"$bin/hwatu" goto --id "$win_id" --until dom "$alpha" >/dev/null
sleep 0.5

hist="$("$bin/hwatu" history alpha 2>&1)" || true
if echo "$hist" | grep -q "alpha.html"; then
    check "records committed navigations" 0
else
    check "records committed navigations" 1 "$hist"
fi

# ---- 3. titles + ranking -------------------------------------------
hist_all="$("$bin/hwatu" history 2>&1)" || true
if echo "$hist_all" | grep -q "Alpha Fixture Page"; then
    check "titles attach to history entries" 0
else
    check "titles attach to history entries" 1 "$hist_all"
fi

# alpha visited 2x, beta 1x: alpha should rank first for empty query.
first="$(echo "$hist_all" | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d["history"][0]["url"])' 2>/dev/null || echo parse-error)"
if [[ "$first" == "$alpha" ]]; then
    check "frecency ranks the double visit first" 0
else
    check "frecency ranks the double visit first" 1 "first=$first"
fi

# ---- 2. headless checks are not recorded ---------------------------
"$bin/hwatu" check "$beta" --until dom --eval "1" >/dev/null 2>&1 || true
hist_after="$("$bin/hwatu" history 2>&1)"
beta_count="$(echo "$hist_after" | python3 -c '
import json,sys
d=json.load(sys.stdin)
hits=[h for h in d["history"] if h["url"].endswith("beta.html")]
print(len(hits))' 2>/dev/null || echo parse-error)"
if [[ "$beta_count" == "1" ]]; then
    check "headless checks not recorded (beta still 1 entry)" 0
else
    check "headless checks not recorded (beta still 1 entry)" 1 "entries=$beta_count"
fi

# ---- 4. no internal pages ------------------------------------------
if echo "$hist_after" | grep -qE "hwatu://|about:blank"; then
    check "no launcher/blank pollution" 1 "internal pages leaked"
else
    check "no launcher/blank pollution" 0
fi

# ---- 5. survives restart -------------------------------------------
"$bin/hwatu" quit >/dev/null 2>&1 || true
sleep 0.5
hist_restart="$("$bin/hwatu" history alpha 2>&1)" || true
if echo "$hist_restart" | grep -q "alpha.html"; then
    check "history survives daemon restart" 0
else
    check "history survives daemon restart" 1 "$hist_restart"
fi

# ---- 6. clear -------------------------------------------------------
cleared="$("$bin/hwatu" history --clear 2>&1)" || true
if echo "$cleared" | grep -qE '"cleared":\s*[1-9]'; then
    check "clear reports removed rows" 0
else
    check "clear reports removed rows" 1 "$cleared"
fi
hist_empty="$("$bin/hwatu" history 2>&1)" || true
empty_count="$(echo "$hist_empty" | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["history"]))' 2>/dev/null || echo parse-error)"
if [[ "$empty_count" == "0" ]]; then
    check "history empty after clear" 0
else
    check "history empty after clear" 1 "count=$empty_count"
fi

echo
echo "test-history: $pass passed, $fail failed"
[[ "$fail" == "0" ]]
