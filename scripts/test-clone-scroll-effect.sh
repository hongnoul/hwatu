#!/usr/bin/env bash
# Focused regression for issue #40: `hwatu clone` should report
# scroll-coupled word highlighting while ignoring a simple scroll-hidden nav.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$root/target/debug"

echo "test-clone-scroll-effect: building debug binaries..." >&2
cargo build --manifest-path "$root/Cargo.toml" >&2

work="$(mktemp -d "${TMPDIR:-/tmp}/hwatu-scroll-effect.XXXXXX")"
export XDG_RUNTIME_DIR="$work/run"
export XDG_STATE_HOME="$work/state"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_STATE_HOME"

cleanup() {
    "$bin/hwatu" quit >/dev/null 2>&1 || true
    if [[ -z "${KEEP_HWATU_SCROLL_EFFECT_TEST:-}" ]]; then
        rm -rf "$work"
    else
        echo "kept $work" >&2
    fi
}
trap cleanup EXIT

fixture_dir="$root/scripts/fixtures"
port="${HWATU_SCROLL_EFFECT_PORT:-$(python3 - <<'PY'
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
urllib.request.urlopen('http://127.0.0.1:$port/clone-scroll-highlight.html', timeout=.2).read(1)
PY
    then
        break
    fi
    sleep .1
done

out="$work/out"
"$bin/hwatu" clone "http://127.0.0.1:$port/clone-scroll-highlight.html" \
    --out "$out" --viewport 1000x720 --no-verify --timeout-ms 60000 >&2

python3 - "$out" <<'PY'
import json, pathlib, sys
out = pathlib.Path(sys.argv[1])
cap = json.loads((out / 'capture.json').read_text())
effects = cap.get('scrollEffects') or []
assert effects, 'capture.json has no scrollEffects'
effect = effects[0]
assert effect.get('kind') == 'scroll-coupled-text-style', effect
assert effect.get('changedTextNodes', 0) >= 5, effect
labels = [sample.get('label') for sample in effect.get('samples', [])]
assert labels == ['entry', 'midpoint', 'exit'], labels
text = ' '.join(sample_el.get('text', '')
                for sample in effect.get('samples', [])
                for sample_el in sample.get('elements', []))
assert 'Scale.' in text and 'Products' not in text and 'Customers' not in text, text
styles = [sample_el.get('style', {})
          for sample in effect.get('samples', [])
          for sample_el in sample.get('elements', [])]
colors = {s.get('color') for s in styles}
opacities = {s.get('opacity') for s in styles}
assert len(colors) >= 2 or len(opacities) >= 2, styles
report = json.loads((out / 'scroll-effects.json').read_text())
assert 'does not replay' in report['envelope'], report
html = (out / 'index.html').read_text()
assert 'see scroll-effects.json' in html, html[-500:]
print('PASS scrollEffects:', effect['changedTextNodes'], 'text nodes, labels=', ','.join(labels))
PY

kill "$server_pid" >/dev/null 2>&1 || true
