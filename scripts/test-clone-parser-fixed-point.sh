#!/usr/bin/env bash
# Focused regression for issue #47: JS-built div-in-p DOMs must serialize
# to a parser fixed point without ejecting line-wrapper descendants.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$root/target/debug"

echo "test-clone-parser-fixed-point: building debug binaries..." >&2
cargo build --manifest-path "$root/Cargo.toml" >&2

work="$(mktemp -d "${TMPDIR:-/tmp}/hwatu-parser-fixed-point.XXXXXX")"
export XDG_RUNTIME_DIR="$work/run"
export XDG_STATE_HOME="$work/state"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_STATE_HOME"

cleanup() {
    "$bin/hwatu" quit >/dev/null 2>&1 || true
    if [[ -z "${KEEP_HWATU_PARSER_FIXED_POINT_TEST:-}" ]]; then
        rm -rf "$work"
    else
        echo "kept $work" >&2
    fi
}
trap cleanup EXIT

fixture_dir="$root/scripts/fixtures"
port="${HWATU_PARSER_FIXED_POINT_PORT:-$(python3 - <<'PY'
import socket
s = socket.socket()
s.bind(('127.0.0.1', 0))
print(s.getsockname()[1])
s.close()
PY
)}"
python3 -m http.server "$port" --directory "$fixture_dir" >"$work/http.log" 2>&1 &
server_pid=$!
for _ in {1..50}; do
    if python3 - <<PY >/dev/null 2>&1
import urllib.request
urllib.request.urlopen('http://127.0.0.1:$port/clone-parser-fixed-point.html', timeout=.2).read(1)
PY
    then
        break
    fi
    sleep .1
done

out="$work/out"
"$bin/hwatu" clone "http://127.0.0.1:$port/clone-parser-fixed-point.html" \
    --out "$out" --viewport 900x600 --no-verify --timeout-ms 60000 >&2

python3 - "$out" <<'PY'
import json, pathlib, re, sys
out = pathlib.Path(sys.argv[1])
cap = json.loads((out / 'capture.json').read_text())
meta = cap.get('parserFixedPoint') or {}
rewritten = meta.get('rewritten') or []
assert meta.get('fixed_point') is True, meta
assert meta.get('reparsed_tag_count_deltas') == [], meta
assert meta.get('injected_css') is True, meta
assert len(rewritten) == 1, rewritten
entry = rewritten[0]
assert entry.get('from') == 'p' and entry.get('to') == 'div', entry
assert entry.get('block_children') == ['div'], entry
assert entry.get('class') == 'AnimatedText', entry

html = (out / 'index.html').read_text()
assert '<p id="animated"' not in html, html
assert 'data-hwatu-was="p"' in html, html
assert 'role="paragraph"' in html, html
assert 'data-hwatu-parser-fix="0"' in html, html
assert re.search(r'id="animated"[^>]*class="AnimatedText"', html), html
assert re.search(r'data-hwatu-was="p"[^>]*>\s*<div class="line">Line <div class="word">Preserved</div>', html), html
assert ':where([data-hwatu-was="p"][role="paragraph"])' in html, html
assert '[data-hwatu-was="p"][role="paragraph"] {' not in html, html
assert '.AnimatedText { margin: 0; }' in html, html
assert html.find('.AnimatedText { margin: 0; }') < html.find(':where([data-hwatu-was="p"][role="paragraph"])'), html

report = json.loads((out / 'report.json').read_text())
report_meta = report.get('parser_fixed_point') or {}
assert report_meta.get('fixed_point') is True, report_meta
assert report_meta.get('reparsed_tag_count_deltas') == [], report_meta
assert report_meta.get('injected_css') is True, report_meta
assert report_meta.get('rewritten') == rewritten, report_meta
print('PASS parser fixed point: rewritten=%d fixed_point=%s css=%s' % (
    len(rewritten), report_meta.get('fixed_point'), report_meta.get('injected_css')))
PY

clone_url="file://$(python3 - "$out/index.html" <<'PY'
import pathlib, sys
print(pathlib.Path(sys.argv[1]).resolve())
PY
)"
"$bin/hwatu" --headless "$clone_url" >/dev/null
clone_id="$("$bin/hwatu" list --json | python3 -c 'import json,sys; d=json.load(sys.stdin); assert len(d)==1,d; print(d[0]["id"])')"
dom_assert_js=$(cat <<'JS'
const animated = document.querySelector('#animated');
if (!animated) throw new Error('#animated missing after clone reparse');
if (animated.tagName !== 'DIV') throw new Error('#animated should reparse as div, got ' + animated.tagName);
if (animated.getAttribute('role') !== 'paragraph') throw new Error('role=paragraph missing');
if (animated.dataset.hwatuWas !== 'p') throw new Error('data-hwatu-was=p missing');
if (!animated.querySelector(':scope > .line > .word')) throw new Error('.line/.word are not descendants of #animated');
if (document.querySelector('main#root > .line, main#root > .word')) throw new Error('line/word wrapper ejected as root sibling');
const margin = getComputedStyle(animated).marginTop + '/' + getComputedStyle(animated).marginBottom;
if (margin !== '0px/0px') throw new Error('class reset margin drifted: ' + margin);
'PASS parsed clone DOM: #animated div owns .line/.word and margin=' + margin;
JS
)
"$bin/hwatu" eval --id "$clone_id" "$dom_assert_js"

kill "$server_pid" >/dev/null 2>&1 || true
