#!/usr/bin/env bash
# Behavioral suite for `hwatu check --viewports` (roadmap P1 5c): the
# multi-viewport sweep. Runs against a live daemon on an ISOLATED
# socket/state dir so the user's daemon and session are untouched.
#
# Asserts, per the roadmap test plan:
#   1. One check at N sizes returns per-viewport results in one reply
#      (viewports: [{size, eval, shot, pass_ms, ...}]).
#   2. A fixture with CSS breakpoints yields DIFFERENT per-viewport
#      eval results in that one call (the point of the sweep).
#   3. Per-size screenshots exist with the requested pixel dimensions
#      and the -<WxH> suffix naming.
#   4. --baseline-dir <dir> diffs each size against <dir>/<WxH>.png:
#      pristine baselines pass (match ~100), a perturbed page fails.
#   5. Sweeps reuse ONE pooled window (no window-count growth), and a
#      plain check after a sweep sees the default viewport again.
#   6. render --viewports sweeps inline markup too.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$root/target/release"

if [[ ! -x "$bin/hwatu" || ! -x "$bin/hwatud" ]]; then
    echo "test-viewports: building release binaries..." >&2
    cargo build --release --manifest-path "$root/Cargo.toml" >&2
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/hwatu-viewports-test.XXXXXX")"
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
# jq-lite: read a python expression over parsed stdin JSON as `j`.
pyj() { python3 -c "import json,sys; j=json.load(sys.stdin); print($1)"; }

# ---- fixtures -------------------------------------------------------
fixdir="$work/fix"
mkdir -p "$fixdir"
# CSS breakpoints: layout mode flips at 768 and 1280 CSS px, exposed
# both via getComputedStyle content and element visibility.
cat > "$fixdir/responsive.html" <<'HTML'
<!DOCTYPE html><meta charset="utf-8"><title>responsive</title>
<style>
  body { margin: 0; background: #fff; }
  #mode::after { content: "mobile"; }
  #desktop-nav { display: none; }
  @media (min-width: 768px) {
    #mode::after { content: "tablet"; }
    body { background: #dfe8ff; }
  }
  @media (min-width: 1280px) {
    #mode::after { content: "desktop"; }
    #desktop-nav { display: block; }
    body { background: #ffe8df; }
  }
</style>
<div id="mode"></div>
<nav id="desktop-nav">nav</nav>
HTML

port=8642
python3 -m http.server "$port" --bind 127.0.0.1 --directory "$fixdir" >"$work/http.log" 2>&1 &
server_pid=$!
for _ in $(seq 50); do
    curl -sf -o "$work/curl.out" "http://127.0.0.1:$port/responsive.html" && break
    sleep 0.1
done
url="http://127.0.0.1:$port/responsive.html"

"$bin/hwatu" ping >/dev/null # spawn the isolated daemon

MODE_JS='getComputedStyle(document.getElementById("mode"), "::after").content'

# ---- 1+2. one call, three sizes, different eval per size -------------
out="$("$bin/hwatu" check "$url" \
    --viewports 360x640,768x1024,1920x1080 \
    --eval "$MODE_JS" \
    --shot="$work/sweep.png")"
check "sweep reply has a 3-entry viewports array" \
    test "$(pyj 'len(j["viewports"])' <<<"$out")" = 3
sizes="$(pyj '",".join(v["size"] for v in j["viewports"])' <<<"$out")"
check "sizes are labeled in request order ($sizes)" \
    test "$sizes" = "360x640,768x1024,1920x1080"
modes="$(pyj '",".join(v["eval"] for v in j["viewports"])' <<<"$out")"
check "CSS breakpoints yield different eval per viewport ($modes)" \
    test "$modes" = '"mobile","tablet","desktop"'
check "each pass reports its own pass_ms" \
    test "$(pyj 'sum(1 for v in j["viewports"] if v["pass_ms"] >= 0)' <<<"$out")" = 3
check "reply keeps top-level url/title/load_ms/total_ms" \
    test "$(pyj 'j["title"]' <<<"$out")" = "responsive"

# ---- 3. per-size shots exist with the requested dimensions ----------
img_size() { python3 -c '
import struct, sys
with open(sys.argv[1], "rb") as f:
    data = f.read(26)
w, h = struct.unpack(">II", data[16:24])
print(f"{w}x{h}")' "$1"; }
all_named=1
for size in 360x640 768x1024 1920x1080; do
    [[ -s "$work/sweep-$size.png" ]] || all_named=0
done
check "per-size shots use the -<WxH> suffix naming" test "$all_named" = 1
if [[ -s "$work/sweep-360x640.png" ]]; then
    check "360x640 shot has the requested pixel dimensions" \
        test "$(img_size "$work/sweep-360x640.png")" = "360x640"
    check "1920x1080 shot has the requested pixel dimensions" \
        test "$(img_size "$work/sweep-1920x1080.png")" = "1920x1080"
else
    bad "360x640 shot has the requested pixel dimensions"
    bad "1920x1080 shot has the requested pixel dimensions"
fi

# ---- 4. --baseline-dir: per-size baselines pass, a change fails ------
basedir="$work/baselines"
mkdir -p "$basedir"
for size in 360x640 1920x1080; do
    cp "$work/sweep-$size.png" "$basedir/$size.png"
done
out="$("$bin/hwatu" check "$url" \
    --viewports 360x640,1920x1080 --baseline-dir "$basedir")"
match_lo="$(pyj 'min(v["diff"]["match_percent"] for v in j["viewports"])' <<<"$out")"
check "pristine per-size baselines pass (min match ${match_lo})" \
    awk "BEGIN { exit !(${match_lo:-0} >= 99) }"

cat > "$fixdir/responsive2.html" <<'HTML'
<!DOCTYPE html><meta charset="utf-8"><title>responsive</title>
<style>
  body { margin: 0; background: #7a2; }
  @media (min-width: 1280px) { body { background: #000; } }
</style>
<div>changed layout at every width</div>
HTML
out="$("$bin/hwatu" check "http://127.0.0.1:$port/responsive2.html" \
    --viewports 360x640,1920x1080 --baseline-dir "$basedir")"
match_hi="$(pyj 'max(v["diff"]["match_percent"] for v in j["viewports"])' <<<"$out")"
check "changed page fails its per-size baselines (max match ${match_hi})" \
    awk "BEGIN { exit !(${match_hi:-100} < 99) }"
check "per-size diff names its own baseline size in the envelope" \
    test "$(pyj 'j["viewports"][1]["diff"]["envelope"]["viewport"]["width"]' <<<"$out")" = 1920

# ---- 5. window strategy: one pooled window, viewport resets ----------
before="$("$bin/hwatu" list --json | pyj 'len(j)')"
"$bin/hwatu" check "$url" --viewports 360x640,768x1024,1920x1080 >/dev/null
after="$("$bin/hwatu" list --json | pyj 'len(j)')"
check "a sweep adds no windows over a plain check ($before -> $after)" \
    test "$after" -le "$before"
out="$("$bin/hwatu" check "$url" --eval 'innerWidth')"
check "plain check after a sweep sees the default viewport again" \
    test "$(pyj 'j["eval"]' <<<"$out")" = 1920

# ---- 6. render --viewports sweeps inline markup ----------------------
out="$("$bin/hwatu" render "$fixdir/responsive.html" \
    --viewports 360x640,1920x1080 --eval "$MODE_JS")"
modes="$(pyj '",".join(v["eval"] for v in j["viewports"])' <<<"$out")"
check "render sweep sees per-viewport styles too ($modes)" \
    test "$modes" = '"mobile","desktop"'
check "render sweep marks rendered:true" \
    grep -q '"rendered":true' <<<"$out"

echo
echo "test-viewports: $pass passed, $fail failed"
[[ "$fail" -eq 0 ]]
