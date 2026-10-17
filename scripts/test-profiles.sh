#!/usr/bin/env bash
# Behavioral suite for profiles (platform item 6): cookie/site-data
# isolation between named profiles. Isolated daemon/state.
#
#   1. Default-session windows share cookies.
#   2. A --profile window does NOT see the default session's cookies.
#   3. Two windows in the same profile share cookies.
#   4. Two different profiles are isolated from each other.
#   5. Profile cookie stores land under profiles/<name>/ on disk.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$root/target/release"

if [[ ! -x "$bin/hwatu" || ! -x "$bin/hwatud" ]]; then
    echo "test-profiles: building release binaries..." >&2
    cargo build --release --manifest-path "$root/Cargo.toml" >&2
fi

# shellcheck source=lib/test-harness.sh
source "$root/scripts/lib/test-harness.sh"
hw_private_env "hwatu-profiles-test"
export XDG_DATA_HOME="$work/data"
export XDG_CACHE_HOME="$work/cache"
mkdir -p "$XDG_DATA_HOME" "$XDG_CACHE_HOME"

hw_set_quit_cmd "$bin/hwatu"
hw_cleanup_install EXIT

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
printf '<!doctype html><title>p</title><body>profile fixture</body>\n' > "$site/page.html"

port=8648
python3 -m http.server "$port" --directory "$site" --bind 127.0.0.1 >/dev/null 2>&1 &
server_pid=$!
hw_track_pid "$server_pid"
for _ in $(seq 50); do
    curl -sf "http://127.0.0.1:$port/page.html" >/dev/null 2>&1 && break
    sleep 0.1
done
url="http://127.0.0.1:$port/page.html"

open_id() { # open_id [--profile name] -> id
    "$bin/hwatu" --headless --no-wait "$@" "$url" >/dev/null 2>&1
    sleep 0.7
    "$bin/hwatu" list --json | python3 -c 'import json,sys; ws=json.load(sys.stdin); print(ws[-1]["id"])'
}
cookies_of() { # cookies_of <id>
    "$bin/hwatu" eval --id "$1" "return document.cookie" 2>&1
}

# ---- 1. default session shares --------------------------------------
a="$(open_id)"
"$bin/hwatu" eval --id "$a" "document.cookie='side=default; path=/'; return 1" >/dev/null
b="$(open_id)"
got="$(cookies_of "$b")"
[[ "$got" == '"side=default"' ]]
check "default-session windows share cookies" $? "got=$got"

# ---- 2. profile isolated from default -------------------------------
p1="$(open_id --profile alice)"
got="$(cookies_of "$p1")"
[[ "$got" == '""' ]]
check "profile window blind to default-session cookies" $? "got=$got"

# ---- 3. same profile shares ------------------------------------------
"$bin/hwatu" eval --id "$p1" "document.cookie='who=alice; path=/'; return 1" >/dev/null
p2="$(open_id --profile alice)"
got="$(cookies_of "$p2")"
[[ "$got" == '"who=alice"' ]]
check "same-profile windows share cookies" $? "got=$got"

# ---- 4. different profiles isolated -----------------------------------
q1="$(open_id --profile bob)"
got="$(cookies_of "$q1")"
[[ "$got" == '""' ]]
check "different profiles are isolated" $? "got=$got"

# ---- 5. per-profile store on disk --------------------------------------
if [[ -d "$XDG_DATA_HOME/hwatud/profiles/alice" && -d "$XDG_DATA_HOME/hwatud/profiles/bob" ]]; then
    check "profile data dirs created under profiles/<name>/" 0
else
    check "profile data dirs created under profiles/<name>/" 1 "$(find "$XDG_DATA_HOME" -maxdepth 3 -type d 2>&1 | head -5 | tr '\n' ' ')"
fi

echo
echo "test-profiles: $pass passed, $fail failed"
[[ "$fail" == "0" ]]
