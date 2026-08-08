#!/usr/bin/env bash
# Behavioral suite for search keywords + quickmarks (roadmap H13).
# Runs against a live daemon on an ISOLATED socket/config dir.
#
#   1. `w <query>` resolves through the keyword's engine template.
#   2. A bare quickmark name goes straight to its URL.
#   3. Plain queries still use the default engine.
#   4. Keyword lines don't corrupt the default engine.
#
# Resolution is asserted via the URL the daemon actually navigates to
# (hwatu list), no network needed beyond the initial DNS-free load
# attempt (pages may fail to load; the URL is set regardless).
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$root/target/release"

if [[ ! -x "$bin/hwatu" || ! -x "$bin/hwatud" ]]; then
    echo "test-search-keywords: building release binaries..." >&2
    cargo build --release --manifest-path "$root/Cargo.toml" >&2
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/hwatu-kw-test.XXXXXX")"
export XDG_RUNTIME_DIR="$work/run"
export XDG_STATE_HOME="$work/state"
export XDG_DATA_HOME="$work/data"
export XDG_CONFIG_HOME="$work/config"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_STATE_HOME" "$XDG_DATA_HOME" "$XDG_CONFIG_HOME/hwatu"

cat > "$XDG_CONFIG_HOME/hwatu/search.conf" <<'CONF'
duckduckgo
w https://en.wikipedia.org/w/index.php?search=%s
CONF
cat > "$XDG_CONFIG_HOME/hwatu/quickmarks.conf" <<'CONF'
news https://news.ycombinator.com/
CONF

cleanup() {
    "$bin/hwatu" quit >/dev/null 2>&1 || true
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

# Open windows without waiting for the (networkless) loads to finish.
url_of() { # url_of <search-input...>
    local out
    out="$("$bin/hwatu" --headless --no-wait "$@" 2>/dev/null)" || true
    "$bin/hwatu" list --json | python3 -c '
import json,sys
ws=json.load(sys.stdin)
print(ws[-1]["url"] if ws else "")'
}

u="$(url_of w rust language)"
if [[ "$u" == "https://en.wikipedia.org/w/index.php?search=rust+language" ]]; then
    check "keyword 'w' routes to wikipedia template" 0
else
    check "keyword 'w' routes to wikipedia template" 1 "url=$u"
fi

u="$(url_of news)"
if [[ "$u" == "https://news.ycombinator.com/" ]]; then
    check "quickmark 'news' goes straight to its URL" 0
else
    check "quickmark 'news' goes straight to its URL" 1 "url=$u"
fi

u="$(url_of how to exit vim)"
if [[ "$u" == "https://duckduckgo.com/?q=how+to+exit+vim" ]]; then
    check "plain query uses the default engine" 0
else
    check "plain query uses the default engine" 1 "url=$u"
fi

u="$(url_of zz unknown keyword search)"
if [[ "$u" == "https://duckduckgo.com/?q=zz+unknown+keyword+search" ]]; then
    check "unknown keyword falls through to default engine" 0
else
    check "unknown keyword falls through to default engine" 1 "url=$u"
fi

echo
echo "test-search-keywords: $pass passed, $fail failed"
[[ "$fail" == "0" ]]
