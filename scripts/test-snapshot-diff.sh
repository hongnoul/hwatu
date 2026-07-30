#!/usr/bin/env bash
# Behavioral suite for `hwatu snapshot --diff` (roadmap P1.3): only
# what changed since the last snapshot. Runs against a live daemon on
# an ISOLATED socket/state dir so the user's daemon and session are
# untouched.
#
# Asserts, per the roadmap spec:
#   1. The first --diff on a window returns the full snapshot with
#      baseline_established:true (and live refs).
#   2. An unchanged page diffs to empty (added/removed/changed all
#      empty, unchanged_count > 0).
#   3. A DOM mutation surfaces only the mutated node.
#   4. Refs in the diff are live handles: click --ref works on them.
#   5. Navigation resets the baseline (next --diff is a new baseline).
#   6. Plain `hwatu snapshot` (no --diff) still returns the full
#      snapshot and does not disturb the diff baseline.
#
# Usage: scripts/test-snapshot-diff.sh
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$root/target/release"

if [[ ! -x "$bin/hwatu" || ! -x "$bin/hwatud" ]]; then
    echo "test-snapshot-diff: building release binaries..." >&2
    cargo build --release --manifest-path "$root/Cargo.toml" >&2
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/hwatu-snapdiff-test.XXXXXX")"
export XDG_RUNTIME_DIR="$work/run"
export XDG_STATE_HOME="$work/state"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_STATE_HOME"

cleanup() {
    "$bin/hwatu" quit >/dev/null 2>&1 || true
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
# jq-less JSON probes (python3 is already a test dependency).
jget() { # jget <json> <python expression over d>
    python3 -c 'import json,sys; d=json.loads(sys.argv[1]); print(eval(sys.argv[2]))' "$1" "$2"
}

# ---- fixtures -------------------------------------------------------
fixdir="$work/fix"
mkdir -p "$fixdir"
cat > "$fixdir/page-a.html" <<'HTML'
<!DOCTYPE html><meta charset="utf-8"><title>diff fixture</title>
<h1>stable heading</h1>
<p id="status">initial</p>
<button id="go">Go</button>
<button id="mutate" onclick="document.getElementById('status').textContent='mutated'">Mutate</button>
HTML
cat > "$fixdir/page-b.html" <<'HTML'
<!DOCTYPE html><meta charset="utf-8"><title>other page</title>
<h1>a different document</h1>
<a id="home" href="page-a.html">back</a>
HTML

"$bin/hwatu" ping >/dev/null # spawn the isolated daemon
"$bin/hwatu" "$fixdir/page-a.html" --headless >/dev/null
"$bin/hwatu" wait-load >/dev/null

# ---- 1. first --diff establishes the baseline ------------------------
out="$("$bin/hwatu" snapshot --diff)"
check "first --diff returns baseline_established:true" \
    grep -q '"baseline_established":true' <<<"$out"
check "first --diff carries the full snapshot (title present)" \
    grep -q '"title":"diff fixture"' <<<"$out"
check "first --diff has interactables with refs" \
    test "$(jget "$out" 'len(d["interactables"])')" -ge 2

# ---- 2. unchanged page -> empty diff ---------------------------------
out="$("$bin/hwatu" snapshot --diff)"
check "unchanged page: no added" test "$(jget "$out" 'len(d["added"])')" -eq 0
check "unchanged page: no removed" test "$(jget "$out" 'len(d["removed"])')" -eq 0
check "unchanged page: no changed" test "$(jget "$out" 'len(d["changed"])')" -eq 0
check "unchanged page: unchanged_count > 0" \
    test "$(jget "$out" 'd["unchanged_count"]')" -gt 0

# ---- 3. DOM mutation -> only the mutated node ------------------------
"$bin/hwatu" eval "document.getElementById('status').textContent = 'updated'" >/dev/null
out="$("$bin/hwatu" snapshot --diff)"
total="$(jget "$out" 'len(d["added"]) + len(d["removed"]) + len(d["changed"])')"
check "text mutation: exactly one removed + one added line (got $total entries)" \
    test "$(jget "$out" 'len(d["removed"])')" -eq 1 -a "$(jget "$out" 'len(d["added"])')" -eq 1
check "text mutation: removed is the old text" \
    grep -q '"text":"initial"' <<<"$out"
check "text mutation: added is the new text" \
    grep -q '"text":"updated"' <<<"$out"
check "text mutation: interactables not reported" \
    test "$(jget "$out" 'len(d["changed"])')" -eq 0

# ---- 4. attribute mutation on an anchored node -> changed ------------
"$bin/hwatu" eval "document.getElementById('go').textContent = 'Really go'" >/dev/null
out="$("$bin/hwatu" snapshot --diff)"
check "button relabel: reported as changed (key button#go)" \
    grep -q '"key":"button#go"' <<<"$out"
ref="$(jget "$out" '[c for c in d["changed"] if c["key"] == "button#go"][0]["new"]["ref"]')"
check "changed node carries a live ref (got $ref)" test "$ref" -ge 0
# The ref must be a live handle: clicking it runs the button's handler.
"$bin/hwatu" eval "document.getElementById('go').onclick = () => { document.title = 'clicked' }; 'wired'" >/dev/null
"$bin/hwatu" snapshot --diff >/dev/null # refresh refs after the eval
"$bin/hwatu" click --ref "$ref" >/dev/null
out="$("$bin/hwatu" eval "document.title")"
check "diff ref is clickable (title becomes 'clicked')" \
    grep -q 'clicked' <<<"$out"

# ---- 5. navigation resets the baseline -------------------------------
"$bin/hwatu" goto "$fixdir/page-b.html" >/dev/null
out="$("$bin/hwatu" snapshot --diff)"
check "post-navigation --diff re-establishes the baseline" \
    grep -q '"baseline_established":true' <<<"$out"
check "post-navigation baseline is the new document" \
    grep -q '"title":"other page"' <<<"$out"
out="$("$bin/hwatu" snapshot --diff)"
check "second post-navigation --diff is an empty diff" \
    test "$(jget "$out" 'len(d["added"]) + len(d["removed"]) + len(d["changed"])')" -eq 0

# ---- 6. plain snapshot still works and leaves the baseline alone -----
out="$("$bin/hwatu" snapshot)"
check "plain snapshot still returns the full page" \
    grep -q '"title":"other page"' <<<"$out"
if grep -q 'baseline_established\|unchanged_count' <<<"$out"; then
    bad "plain snapshot has no diff fields"
else
    ok "plain snapshot has no diff fields"
fi
out="$("$bin/hwatu" snapshot --diff)"
check "plain snapshot did not disturb the diff baseline" \
    test "$(jget "$out" 'len(d["added"]) + len(d["removed"]) + len(d["changed"])')" -eq 0

echo
echo "test-snapshot-diff: $pass passed, $fail failed"
[[ "$fail" -eq 0 ]]
