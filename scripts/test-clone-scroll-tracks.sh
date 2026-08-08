#!/usr/bin/env bash
# Focused regression for issue #46: clone scroll-effects should serialize and
# replay visual scrub tracks, stable pins, and threshold-triggered time tracks.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$root/target/debug"

echo "test-clone-scroll-tracks: building debug binaries..." >&2
cargo build --manifest-path "$root/Cargo.toml" >&2

work="$(mktemp -d "${TMPDIR:-/tmp}/hwatu-scroll-tracks.XXXXXX")"
export XDG_RUNTIME_DIR="$work/run"
export XDG_STATE_HOME="$work/state"
export XDG_CONFIG_HOME="$work/config"
export XDG_CACHE_HOME="$work/cache"
export XDG_DATA_HOME="$work/data"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_STATE_HOME" "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME" "$XDG_DATA_HOME"
server_pid=""

cleanup() {
    "$bin/hwatu" quit >/dev/null 2>&1 || true
    if [[ -n "${server_pid:-}" ]]; then
        kill "$server_pid" >/dev/null 2>&1 || true
        wait "$server_pid" >/dev/null 2>&1 || true
    fi
    if [[ -z "${KEEP_HWATU_SCROLL_TRACKS_TEST:-}" ]]; then
        rm -rf "$work"
    else
        echo "kept $work" >&2
    fi
}
trap cleanup EXIT

fixture_dir="$root/scripts/fixtures"
port="${HWATU_SCROLL_TRACKS_PORT:-$(python3 - <<'PY'
import socket
s = socket.socket(); s.bind(('127.0.0.1', 0)); print(s.getsockname()[1]); s.close()
PY
)}"
python3 -m http.server "$port" --directory "$fixture_dir" >"$work/http.log" 2>&1 &
server_pid=$!
for _ in {1..50}; do
    if python3 - <<PY >/dev/null 2>&1
import urllib.request
urllib.request.urlopen('http://127.0.0.1:$port/clone-scroll-tracks.html', timeout=.2).read(1)
PY
    then break; fi
    sleep .1
done

out="$work/out"
"$bin/hwatu" clone "http://127.0.0.1:$port/clone-scroll-tracks.html" \
    --out "$out" --viewport 1000x720 --no-verify --timeout-ms 60000 >&2

python3 - "$out" <<'PY'
import json, pathlib, sys
out = pathlib.Path(sys.argv[1])
cap = json.loads((out / 'capture.json').read_text())
effects = cap.get('scrollEffects') or []
kinds = {e.get('kind') for e in effects}
assert 'scroll-coupled-visual-style' in kinds, kinds
assert 'scroll-pin' in kinds, kinds
assert 'scroll-triggered-time-style' in kinds, kinds
visual = next(e for e in effects if e.get('kind') == 'scroll-coupled-visual-style')
assert visual.get('from', {}).get('clipPath') != visual.get('to', {}).get('clipPath'), visual
assert visual.get('from', {}).get('transform') != visual.get('to', {}).get('transform'), visual
pin = next(e for e in effects if e.get('kind') == 'scroll-pin')
assert pin.get('endY', 0) > pin.get('startY', 0), pin
assert 'desiredViewportTop' in pin and 'pinnedSelector' in pin, pin
assert pin.get('pinnedSelector') == '#pin-root', pin
time = next(e for e in effects if e.get('kind') == 'scroll-triggered-time-style')
assert time.get('before') and time.get('after') and time.get('triggerY', -1) >= 0, time
report = json.loads((out / 'scroll-effects.json').read_text())
reported = {e.get('kind'): e for e in report['effects']}
for kind in ('scroll-coupled-visual-style', 'scroll-pin', 'scroll-triggered-time-style'):
    assert reported[kind].get('replay') == 'replayed', reported[kind]
html = (out / 'index.html').read_text()
assert 'hwatu-scroll-tracks-replay' in html, 'new replay runtime missing'
for marker in ('translate3d(0,', 'clipPath', 'hwatuTimeState', 'direct serialized scroll track runtime'):
    assert marker in html or marker in json.dumps(report), marker
for marker in ('baseTransform', 'computedTransform', "s.child.style.transform = s.baseline"):
    assert marker in html, marker
print('PASS scroll track kinds:', ','.join(sorted(kinds)))
PY

clone_url="file://$out/index.html"
"$bin/hwatu" --headless "$clone_url" >/dev/null
for _ in {1..50}; do
    "$bin/hwatu" list --json >"$work/list.json"
    clone_id="$(python3 - "$clone_url" "$work/list.json" <<'PY'
import json, sys
want = sys.argv[1]
path = sys.argv[2]
try:
    wins = json.load(open(path))
except Exception:
    wins = []
for w in wins:
    if w.get('url') == want:
        print(w.get('id'))
        break
PY
)"
    [[ -n "${clone_id:-}" ]] && break
    sleep .1
done
[[ -n "${clone_id:-}" ]] || { echo "failed to find clone window" >&2; exit 1; }

"$bin/hwatu" eval --id "$clone_id" --timeout-ms 20000 "
return (async () => {
async function settle(y) {
  scrollTo(0, y);
  dispatchEvent(new Event('scroll'));
  document.dispatchEvent(new Event('scroll'));
  await new Promise(r => setTimeout(r, 450));
}
function snap() {
  const visual = document.querySelector('#visual');
  const timed = document.querySelector('#timed');
  const pin = document.querySelector('#pin-root');
  const vs = getComputedStyle(visual);
  const ts = getComputedStyle(timed);
  return {
    y: scrollY,
    visual: { opacity: vs.opacity, transform: vs.transform, clipPath: vs.clipPath },
    timed: { state: timed.dataset.hwatuTimeState || '', opacity: ts.opacity, transform: ts.transform },
    pin: { top: pin.getBoundingClientRect().top, transform: getComputedStyle(pin).transform }
  };
}
await settle(0); const a = snap();
await settle(1150); const b = snap();
await settle(1700); const p1 = snap();
await settle(2300); const p2 = snap();
await settle(0); const t0 = snap();
await settle(3000); const t1 = snap();
return { a, b, p1, p2, t0, t1 };
})()
" >"$work/runtime.json"

python3 - "$work/runtime.json" <<'PY'
import json, sys
raw = open(sys.argv[1]).read()
data = json.loads(raw)
if isinstance(data, dict) and 'value' in data:
    data = data['value']
assert data['a']['visual']['opacity'] != data['b']['visual']['opacity'], data
assert data['a']['visual']['transform'] != data['b']['visual']['transform'], data
assert data['a']['visual']['clipPath'] != data['b']['visual']['clipPath'], data
assert abs(data['p1']['pin']['top'] - data['p2']['pin']['top']) <= 8, data
assert data['p1']['pin']['transform'] != 'none' and data['p2']['pin']['transform'] != 'none', data
assert data['t0']['timed']['state'] in ('', 'out'), data
assert data['t1']['timed']['state'] == 'in', data
print('PASS runtime replay behavior: visual changed, pin stable, time state flipped')
PY
