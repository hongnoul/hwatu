#!/usr/bin/env bash
# Capture the real, machine-readable evidence used by killer-demo-v2.
#
# Usage: capture-v2.sh [output-directory]
# The default output is gitignored by scripts/demo/.gitignore.
set -euo pipefail
IFS=$'\n\t'
umask 077

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
DEMO_DIR=$(cd "$HERE/.." && pwd)
REPO_ROOT=$(cd "$DEMO_DIR/../.." && pwd)
OUT=${1:-"$DEMO_DIR/tmp-capture-v2"}
REF_DIR=${HWATU_DEMO_REFERENCE_DIR:-"$DEMO_DIR/reference"}
CHECKPOINT_ROOT=${HWATU_DEMO_CHECKPOINT_DIR:-"$DEMO_DIR/checkpoints"}
HWATU_BIN=${HWATU_BIN:-"$REPO_ROOT/target/release/hwatu"}
WIDTH=${HWATU_DEMO_WIDTH:-1600}
HEIGHT=${HWATU_DEMO_HEIGHT:-900}
SCROLL_PERCENT=${HWATU_DEMO_SCROLL_PERCENT:-75}
CLOCK_EPOCH_MS=${HWATU_CLOCK_EPOCH_MS:-1784872800000}

fail() { printf 'capture-v2: %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"; }
fixture() { [ -r "$1/index.html" ] || fail "$2 fixture has no readable index.html: $1"; }
pick_port() { python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'; }

need curl; need jq; need python3; need sha256sum; need Xvfb
[ -x "$HWATU_BIN" ] || fail "hwatu binary not found at $HWATU_BIN (run cargo build --release)"
fixture "$REF_DIR" reference
[ -d "$CHECKPOINT_ROOT" ] || fail "checkpoint directory not found: $CHECKPOINT_ROOT"

# Explicit overrides win. Otherwise the honest bookends are the first and last
# lexically sorted checkpoint directories, matching film.sh's bookends mode.
mapfile -d '' CHECKPOINTS < <(find "$CHECKPOINT_ROOT" -mindepth 1 -maxdepth 1 -type d -print0 | sort -z)
((${#CHECKPOINTS[@]} >= 2)) || fail "need at least two checkpoint directories in $CHECKPOINT_ROOT"
BOOKEND_DIR=${HWATU_DEMO_BOOKEND_DIR:-${CHECKPOINTS[0]}}
FINAL_DIR=${HWATU_DEMO_FINAL_DIR:-${CHECKPOINTS[${#CHECKPOINTS[@]}-1]}}
fixture "$BOOKEND_DIR" bookend
fixture "$FINAL_DIR" final

OUT=$(mkdir -p "$OUT" && cd "$OUT" && pwd)
WORK=$(mktemp -d "${TMPDIR:-/tmp}/hwatu-capture-v2.XXXXXX")
RT="$WORK/runtime"; mkdir -m 700 "$RT"
PIDS=()
REF_ID= BUILD_ID=
cleanup() {
  if [ -n "$REF_ID" ]; then XDG_RUNTIME_DIR="$RT" "$HWATU_BIN" close "$REF_ID" >/dev/null 2>&1 || true; fi
  if [ -n "$BUILD_ID" ]; then XDG_RUNTIME_DIR="$RT" "$HWATU_BIN" close "$BUILD_ID" >/dev/null 2>&1 || true; fi
  XDG_RUNTIME_DIR="$RT" "$HWATU_BIN" quit >/dev/null 2>&1 || true
  for pid in "${PIDS[@]:-}"; do kill "$pid" >/dev/null 2>&1 || true; done
  for pid in "${PIDS[@]:-}"; do wait "$pid" >/dev/null 2>&1 || true; done
  rm -rf "$WORK"
}
trap cleanup EXIT HUP INT TERM

# Build in a sibling directory, then atomically replace OUT. Failed runs never
# leave a plausible mixed-generation evidence set behind.
NEXT="$WORK/output"; mkdir "$NEXT"
REF_PORT=$(pick_port); BUILD_PORT=$(pick_port)
python3 -m http.server "$REF_PORT" --bind 127.0.0.1 --directory "$REF_DIR" >"$WORK/ref-server.log" 2>&1 & PIDS+=("$!")
python3 -m http.server "$BUILD_PORT" --bind 127.0.0.1 --directory "$CHECKPOINT_ROOT" >"$WORK/build-server.log" 2>&1 & PIDS+=("$!")
for _ in $(seq 1 100); do
  curl -fs -o /dev/null "http://127.0.0.1:$REF_PORT/" 2>/dev/null \
    && curl -fs -o /dev/null "http://127.0.0.1:$BUILD_PORT/$(basename "$BOOKEND_DIR")/" 2>/dev/null && break
  sleep 0.1
done
curl -fsS -o /dev/null "http://127.0.0.1:$REF_PORT/" || fail "reference server did not become ready"

XVFB_DISPLAY=":$((400 + RANDOM % 400))"
while [ -e "/tmp/.X11-unix/X${XVFB_DISPLAY#:}" ]; do XVFB_DISPLAY=":$((400 + RANDOM % 400))"; done
Xvfb "$XVFB_DISPLAY" -screen 0 "$((WIDTH + 80))x$((HEIGHT + 80))x24" -nolisten tcp >"$WORK/xvfb.log" 2>&1 & PIDS+=("$!")
for _ in $(seq 1 100); do [ -e "/tmp/.X11-unix/X${XVFB_DISPLAY#:}" ] && break; sleep 0.05; done
[ -e "/tmp/.X11-unix/X${XVFB_DISPLAY#:}" ] || fail "Xvfb did not become ready"

export XDG_RUNTIME_DIR="$RT" GDK_BACKEND=x11 DISPLAY="$XVFB_DISPLAY" WAYLAND_DISPLAY=
export TZ=UTC HWATU_CLOCK_EPOCH_MS="$CLOCK_EPOCH_MS"
hw() { "$HWATU_BIN" "$@"; }

# Seed before navigation so both realms receive identical deterministic clocks
# and randomness. Query strings prevent stale WebKit HTTP cache across reruns.
SEED='data:text/html,%3Ctitle%3Ehwatu-v2-seed%3C%2Ftitle%3E'
REF_ID=$(hw --headless --json "$SEED" | jq -er '.id')
BUILD_ID=$(hw --headless --json "$SEED" | jq -er '.id')
for id in "$REF_ID" "$BUILD_ID"; do
  hw wait-load --id "$id" --timeout-ms 30000 >/dev/null
  hw clock --id "$id" seed 1 >/dev/null
  hw resize --id "$id" "${WIDTH}x${HEIGHT}" >"$NEXT/viewport-$id.json"
done
REF_URL="http://127.0.0.1:$REF_PORT/?capture=v2"
hw goto --id "$REF_ID" "$REF_URL" >/dev/null
hw wait-load --id "$REF_ID" --timeout-ms 60000 >/dev/null
hw eval --id "$REF_ID" 'await document.fonts.ready; return document.fonts.status' >/dev/null
hw clock --id "$REF_ID" set 0 >"$NEXT/reference-clock.json"

capture_checkpoint() {
  local label=$1 dir=$2 slug url scroll_js
  slug=$(basename "$dir")
  url="http://127.0.0.1:$BUILD_PORT/$slug/?capture=v2-$label"
  hw goto --id "$BUILD_ID" "$url" >/dev/null
  hw wait-load --id "$BUILD_ID" --timeout-ms 60000 >/dev/null
  hw eval --id "$BUILD_ID" 'await document.fonts.ready; return document.fonts.status' >/dev/null
  hw clock --id "$BUILD_ID" set 0 >"$NEXT/$label-clock.json"
  scroll_js="const m=Math.max(0,document.documentElement.scrollHeight-innerHeight); window.scrollTo(0,Math.round(m*$SCROLL_PERCENT/100)); return {x:scrollX,y:scrollY,max_y:m}"
  hw eval --id "$REF_ID" "$scroll_js" >"$NEXT/$label-reference-scroll.json"
  hw eval --id "$BUILD_ID" "$scroll_js" >"$NEXT/$label-build-scroll.json"
  hw shot --id "$REF_ID" "$NEXT/$label-reference.png" >/dev/null
  hw shot --id "$BUILD_ID" "$NEXT/$label-build.png" >/dev/null
  hw diff --id "$BUILD_ID" --other "$REF_ID" --tolerance 0 \
    --heatmap "$NEXT/$label-heatmap.png" >"$NEXT/$label-diff.json"
}

capture_checkpoint bookend "$BOOKEND_DIR"
capture_checkpoint final "$FINAL_DIR"

# Motion is captured from both real renders. The 0 -> 50 -> 80 -> 50 sequence
# proves this is a scrubbed timeline, not merely disabled animation. Take the
# repeated 50% frame without advancing time and preserve every seek reply.
for id in "$REF_ID" "$BUILD_ID"; do
  # Flush fixture timers/rAF once under virtual time so animations created by
  # startup JavaScript exist before inventory and seek.
  hw clock --id "$id" step 1000 >/dev/null
done
hw motion --id "$REF_ID" >"$NEXT/motion-reference.json"
hw motion --id "$BUILD_ID" >"$NEXT/motion-build.json"
for side in reference build; do
  if [ "$side" = reference ]; then id=$REF_ID; else id=$BUILD_ID; fi
  hw seek --id "$id" --progress 0 >"$NEXT/seek-$side-0.json"
  hw shot --id "$id" "$NEXT/motion-$side-0.png" >/dev/null
  hw seek --id "$id" --progress 0.5 >"$NEXT/seek-$side-50-a.json"
  hw shot --id "$id" "$NEXT/motion-$side-50-a.png" >/dev/null
  hw seek --id "$id" --progress 0.8 >"$NEXT/seek-$side-80.json"
  hw shot --id "$id" "$NEXT/motion-$side-80.png" >/dev/null
  hw seek --id "$id" --progress 0.5 >"$NEXT/seek-$side-50-b.json"
  hw shot --id "$id" "$NEXT/motion-$side-50-b.png" >/dev/null
done
(
  cd "$NEXT"
  sha256sum motion-*-50-a.png motion-*-50-b.png > repeat-hashes.sha256
)

# Give the session an obvious state chip independent of fixture markup. The
# focus transition must preserve both this typed value and the scroll position.
HANDOFF_JS='let e=document.querySelector("#hwatu-v2-handoff-state"); if(!e){e=document.createElement("input");e.id="hwatu-v2-handoff-state";e.value="preserved: exact session";Object.assign(e.style,{position:"fixed",top:"24px",right:"24px",zIndex:"2147483647",font:"700 24px monospace",padding:"12px",width:"390px"});document.body.append(e)}; return {value:e.value,x:scrollX,y:scrollY,url:location.href,title:document.title}'
hw list --json >"$NEXT/handoff-headless.json"
hw eval --id "$BUILD_ID" "$HANDOFF_JS" >"$NEXT/handoff-state-before.json"
hw shot --id "$BUILD_ID" "$NEXT/handoff-before.png" >/dev/null
hw focus "$BUILD_ID" >/dev/null
printf '{"action":"focus","id":%s,"ok":true}\n' "$BUILD_ID" >"$NEXT/handoff-focus.json"
hw list --json >"$NEXT/handoff-live.json"
hw eval --id "$BUILD_ID" "$HANDOFF_JS" >"$NEXT/handoff-state-after.json"
hw shot --id "$BUILD_ID" "$NEXT/handoff-after.png" >/dev/null

# Validate and assemble a path-relative manifest. Exact numeric values are read
# from hwatu JSON rather than duplicated as presentation claims.
python3 - "$NEXT" "$BOOKEND_DIR" "$FINAL_DIR" "$HWATU_BIN" "$WIDTH" "$HEIGHT" "$SCROLL_PERCENT" <<'PY'
import hashlib, json, pathlib, struct, sys
root = pathlib.Path(sys.argv[1])
bookend, final, binary = sys.argv[2:5]
width, height, scroll = map(int, sys.argv[5:8])
expected = [
 "bookend-reference.png", "bookend-build.png", "bookend-heatmap.png", "bookend-diff.json",
 "final-reference.png", "final-build.png", "final-heatmap.png", "final-diff.json",
 "motion-reference.json", "motion-build.json",
 "seek-reference-0.json", "seek-reference-50-a.json", "seek-reference-80.json", "seek-reference-50-b.json",
 "seek-build-0.json", "seek-build-50-a.json", "seek-build-80.json", "seek-build-50-b.json",
 "motion-reference-0.png", "motion-reference-80.png", "motion-build-0.png", "motion-build-80.png",
 "motion-reference-50-a.png", "motion-reference-50-b.png",
 "motion-build-50-a.png", "motion-build-50-b.png", "repeat-hashes.sha256",
 "handoff-headless.json", "handoff-focus.json", "handoff-live.json",
 "handoff-state-before.json", "handoff-state-after.json", "handoff-before.png", "handoff-after.png",
]
for name in expected:
    p = root / name
    if not p.is_file() or p.stat().st_size == 0: raise SystemExit(f"missing/empty expected output: {name}")
for p in root.glob("*.png"):
    with p.open("rb") as f:
        if f.read(8) != b"\x89PNG\r\n\x1a\n": raise SystemExit(f"not PNG: {p.name}")
        length = struct.unpack(">I", f.read(4))[0]
        if f.read(4) != b"IHDR" or length != 13: raise SystemExit(f"bad PNG header: {p.name}")
        w, h = struct.unpack(">II", f.read(8))
        # A focused window adopts the isolated Xvfb window manager's default
        # allocation. All comparison and motion assets must remain master-size.
        if not p.name.startswith("handoff-") and (w < width or h < height):
            raise SystemExit(f"undersized PNG {p.name}: {w}x{h}")
json_files = sorted(root.glob("*.json"))
values = {}
for p in json_files:
    try: values[p.name] = json.loads(p.read_text())
    except Exception as e: raise SystemExit(f"invalid JSON {p.name}: {e}")
for label in ("bookend", "final"):
    d = values[f"{label}-diff.json"]
    if not isinstance(d.get("match_percent"), (int,float)) or not 0 <= d["match_percent"] <= 100:
        raise SystemExit(f"invalid match_percent in {label}-diff.json")
    if not (root / f"{label}-heatmap.png").is_file(): raise SystemExit(f"missing {label} heatmap")
for side in ("reference", "build"):
    inventory = values[f"motion-{side}.json"]
    declared = len(inventory.get("animations", [])) + len(inventory.get("declared", []))
    if declared < 1: raise SystemExit(f"no motion inventory found for {side}")
    for point in ("0", "50-a", "80", "50-b"):
        seek = values[f"seek-{side}-{point}.json"]
        if seek.get("resumed") is not False:
            raise SystemExit(f"seek unexpectedly resumed {side} animation at {point}")
    a = root / f"motion-{side}-50-a.png"; b = root / f"motion-{side}-50-b.png"
    if a.read_bytes() != b.read_bytes(): raise SystemExit(f"{side} repeat frames are not byte-identical")
if values["handoff-state-before.json"] != values["handoff-state-after.json"]:
    raise SystemExit("focus changed the page URL/scroll/title state")
def sha(p): return hashlib.sha256(p.read_bytes()).hexdigest()
assets = {p.name: {"bytes": p.stat().st_size, "sha256": sha(p)} for p in sorted(root.iterdir()) if p.is_file()}
manifest = {
 "schema": "hwatu.demo.capture-v2/1", "viewport": {"width": width, "height": height},
 "scroll_percent": scroll, "fixtures": {"reference": "scripts/demo/reference", "bookend": pathlib.Path(bookend).name, "final": pathlib.Path(final).name},
 "commands": {"binary": binary, "capture": ["shot", "diff --tolerance 0 --heatmap", "motion", "seek --progress 0.5", "focus", "list --json"]},
 "measurements": {"bookend": values["bookend-diff.json"], "final": values["final-diff.json"]},
 "motion": {"reference": values["motion-reference.json"], "build": values["motion-build.json"], "progress": 0.5,
            "repeat": "byte-identical", "hashes": {s: sha(root/f"motion-{s}-50-a.png") for s in ("reference","build")}},
 "handoff": {"session_id": values["handoff-state-before.json"], "state_preserved": True},
 "assets": assets,
}
(root / "evidence-manifest.json").write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
PY

# Replace old evidence only after complete validation, preserving OUT itself.
find "$OUT" -mindepth 1 -maxdepth 1 -exec rm -rf {} +
cp -a "$NEXT"/. "$OUT"/
jq -e '.schema == "hwatu.demo.capture-v2/1" and (.assets|length >= 20)' "$OUT/evidence-manifest.json" >/dev/null
printf 'capture-v2 evidence: %s\n' "$OUT"
printf 'bookend match: %s%%\n' "$(jq -r .measurements.bookend.match_percent "$OUT/evidence-manifest.json")"
printf 'final match:    %s%%\n' "$(jq -r .measurements.final.match_percent "$OUT/evidence-manifest.json")"
