#!/usr/bin/env bash
# Behavioral regressions for scroll-aware, multi-point `expect --visible`.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$root/target/release"

if [[ ! -x "$bin/hwatu" || ! -x "$bin/hwatud" ]]; then
    echo "test-expect-visible: building release binaries..." >&2
    cargo build --release --manifest-path "$root/Cargo.toml" >&2
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/hwatu-expect-visible-test.XXXXXX")"
export XDG_RUNTIME_DIR="$work/run"
export XDG_STATE_HOME="$work/state"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_STATE_HOME"

daemon_pid=""
cleanup() {
    "$bin/hwatu" quit >/dev/null 2>&1 || true
    [[ -n "$daemon_pid" ]] && kill "$daemon_pid" 2>/dev/null || true
    rm -rf "$work"
}
trap cleanup EXIT

"$bin/hwatud" >/dev/null 2>&1 &
daemon_pid=$!
for _ in $(seq 100); do
    [[ -S "$XDG_RUNTIME_DIR/hwatu.sock" ]] && break
    sleep 0.1
done
"$bin/hwatu" ping >/dev/null

page="$work/visible.html"
cat >"$page" <<'HTML'
<!doctype html>
<style>
  body { margin: 0; }
  #target { position: absolute; top: 1200px; left: 40px; width: 240px; height: 80px; }
  #cover { display: none; position: absolute; z-index: 10; background: #111; color: white; }
</style>
<button id="target">Target</button>
<div id="cover">sticky cover</div>
HTML

id="$($bin/hwatu --headless "file://$page" | sed -n 's/^window \([0-9][0-9]*\).*/\1/p')"
"$bin/hwatu" wait-load --id "$id" --until dom >/dev/null

# Fully off-screen targets are scrolled into view for hit testing, but the
# inspection is observational: the caller's scroll position is restored
# afterwards, so scrollY must remain 0 while the diagnostics report the
# internal scroll happened and was undone.
out="$("$bin/hwatu" expect --id "$id" '#target' --visible --timeout-ms 250)"
scroll_y="$($bin/hwatu eval --id "$id" 'scrollY')"
python3 - "$out" "$scroll_y" <<'PY'
import json, sys
vis = json.loads(sys.argv[1])["visibility"]
assert vis["scrolled"] is True, vis
assert vis["scroll_restored"] is True, vis
assert json.loads(sys.argv[2]) == 0, sys.argv[2]
PY

# Cover only the target's top edge. A center-only test would pass, while
# the top-corner samples must identify the partial overlap. The cover is
# absolutely positioned in document coordinates so it still overlaps the
# target during the inspector's internal (restored) scroll-into-view.
"$bin/hwatu" eval --id "$id" 'const t=document.querySelector("#target").getBoundingClientRect(); const c=document.querySelector("#cover"); Object.assign(c.style,{display:"block",left:(t.left+scrollX)+"px",top:(t.top+scrollY)+"px",width:t.width+"px",height:"16px"}); return true;' >/dev/null
if "$bin/hwatu" expect --id "$id" '#target' --visible --timeout-ms 0 >"$work/out" 2>&1; then
    echo "FAIL - partially covered element passed --visible" >&2
    cat "$work/out" >&2
    exit 1
fi
grep -Eq 'top-(left|right) point covered by <div#cover>' "$work/out"

# Removing the overlay restores visibility.
"$bin/hwatu" eval --id "$id" 'document.querySelector("#cover").style.display="none"' >/dev/null
"$bin/hwatu" expect --id "$id" '#target' --visible --timeout-ms 250 >/dev/null

echo "test-expect-visible: 3 passed, 0 failed"
