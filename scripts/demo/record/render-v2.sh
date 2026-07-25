#!/usr/bin/env bash
# Record and render the deterministic killer-demo v2 composition.
#
# Normal invocation (compose-v2.html is already served):
#   scripts/demo/record/render-v2.sh \
#     --url http://127.0.0.1:8000/compose-v2.html \
#     --evidence scripts/demo/evidence-v2/evidence-manifest.json \
#     --out-dir /tmp/hwatu-demo-v2
#
# Render an existing 1600x900, 20-22 second placeholder/take:
#   scripts/demo/record/render-v2.sh --source /tmp/take.mp4 \
#     --evidence /tmp/evidence-manifest.json --out-dir /tmp/hwatu-demo-v2
#
# This script never uploads or publishes. validate-v2.sh is the final gate.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STAGE="$HERE/stage.sh"
URL=""
SOURCE=""
EVIDENCE=""
OUT_DIR=""
CAPTURE_SECONDS="${HWATU_DEMO_V2_SECONDS:-20.5}"
WEBP_FPS="${HWATU_DEMO_V2_WEBP_FPS:-10}"

usage() {
  sed -n '2,15p' "$0"
}

die() { printf 'render-v2: %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"; }

while (($#)); do
  case "$1" in
    --url) URL="${2:?--url requires a value}"; shift 2 ;;
    --source) SOURCE="${2:?--source requires a value}"; shift 2 ;;
    --evidence) EVIDENCE="${2:?--evidence requires a value}"; shift 2 ;;
    --out-dir) OUT_DIR="${2:?--out-dir requires a value}"; shift 2 ;;
    --seconds) CAPTURE_SECONDS="${2:?--seconds requires a value}"; shift 2 ;;
    --webp-fps) WEBP_FPS="${2:?--webp-fps requires a value}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done

[[ -n "$EVIDENCE" && -f "$EVIDENCE" ]] || die "--evidence must name the captured evidence manifest"
[[ -n "$OUT_DIR" ]] || die "--out-dir is required"
[[ -z "$SOURCE" || -z "$URL" ]] || die "use exactly one of --source and --url"
[[ -n "$SOURCE" || -n "$URL" ]] || die "one of --source or --url is required"
awk -v d="$CAPTURE_SECONDS" 'BEGIN { exit !(d >= 20 && d <= 22) }' || die "--seconds must be 20-22"
[[ "$WEBP_FPS" =~ ^[1-9][0-9]*$ ]] || die "--webp-fps must be a positive integer"
need ffmpeg
need ffprobe
need python3
python3 -c 'from PIL import Image' 2>/dev/null || die "Pillow is required"
mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"
EVIDENCE="$(realpath "$EVIDENCE")"
RAW="$OUT_DIR/demo-v2.raw.mp4"
MP4="$OUT_DIR/demo-v2.mp4"
WEBP="$OUT_DIR/demo-v2.webp"
SHEET="$OUT_DIR/demo-v2-contact-sheet.png"
MANIFEST="$OUT_DIR/demo-v2-render-manifest.json"

probe_raw() {
  local dimensions duration
  dimensions="$(ffprobe -v error -select_streams v:0 -show_entries stream=width,height -of csv=p=0:s=x "$RAW")"
  duration="$(ffprobe -v error -show_entries format=duration -of default=nw=1:nk=1 "$RAW")"
  [[ "$dimensions" == "1600x900" ]] || die "raw take is $dimensions, expected 1600x900"
  awk -v d="$duration" 'BEGIN { exit !(d >= 20 && d <= 22) }' || die "raw duration $duration is outside 20-22 seconds"
}

record() {
  need hwatu
  need swaymsg
  need wf-recorder
  [[ -x "$STAGE" ]] || die "stage.sh is not executable"
  local stage_dir="${HWATU_DEMO_STAGE_DIR:-/tmp/hwatu-demo-stage}"
  local runtime="$stage_dir/run"
  local session_id=""
  local paused_url play_url
  local -a shot_urls
  cleanup() {
    "$STAGE" stoprec >/dev/null 2>&1 || true
    "$STAGE" down >/dev/null 2>&1 || true
  }
  trap cleanup EXIT INT TERM
  HWATU_DEMO_RES=1600x900 "$STAGE" up
  export XDG_RUNTIME_DIR="$runtime" XDG_CONFIG_HOME="$stage_dir/config"
  export XDG_CACHE_HOME="$stage_dir/cache" XDG_STATE_HOME="$stage_dir/state"
  export XDG_DATA_HOME="$stage_dir/data" WAYLAND_DISPLAY=wayland-1
  export SWAYSOCK
  SWAYSOCK="$(find "$runtime" -maxdepth 1 -name 'sway-ipc.*.sock' -print -quit)"
  [[ -n "$SWAYSOCK" ]] || die "staged sway IPC socket missing"
  swaymsg 'output HEADLESS-1 mode 1600x900' >/dev/null
  readarray -t shot_urls < <(python3 - "$URL" <<'PY'
import sys
from urllib.parse import parse_qsl, urlencode, urlsplit, urlunsplit
parts = urlsplit(sys.argv[1])
query = [(key, value) for key, value in parse_qsl(parts.query, keep_blank_values=True)
         if key not in {"t", "autoplay"}]
def with_query(extra):
    return urlunsplit(parts._replace(query=urlencode(query + extra)))
print(with_query([("t", "0")]))
print(with_query([("autoplay", "1")]))
PY
  )
  paused_url="${shot_urls[0]}"
  play_url="${shot_urls[1]}"
  # Load a paused deterministic first frame before rolling.
  session_id="$(hwatu --headless --json "$paused_url" | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')"
  hwatu wait-load --id "$session_id" --timeout-ms 20000 >/dev/null
  hwatu focus "$session_id" >/dev/null
  swaymsg '[app_id="dev.hwatu.hwatud"] fullscreen enable' >/dev/null
  sleep 0.5
  "$STAGE" rec "$RAW" >/dev/null
  # Navigation is the clapboard. The composition owns its deterministic clock.
  hwatu goto --id "$session_id" "$play_url" >/dev/null
  sleep "$CAPTURE_SECONDS"
  "$STAGE" stoprec >/dev/null
  trap - EXIT INT TERM
  "$STAGE" down >/dev/null
}

if [[ -n "$SOURCE" ]]; then
  [[ -f "$SOURCE" ]] || die "source does not exist: $SOURCE"
  cp -- "$(realpath "$SOURCE")" "$RAW"
else
  record
fi
probe_raw

# Normalize once. All delivered artifacts derive from this exact 1280x720 cut.
ffmpeg -y -v error -i "$RAW" -t "$CAPTURE_SECONDS" \
  -vf 'scale=1280:720:flags=lanczos,setsar=1' -an -c:v libx264 \
  -pix_fmt yuv420p -crf 18 -preset slow -movflags +faststart "$MP4"
ffmpeg -y -v error -i "$MP4" -vf "fps=$WEBP_FPS" -an \
  -c:v libwebp -lossless 0 -q:v 72 -compression_level 6 -loop 0 "$WEBP"

# Eight timestamped frames, 2x4 at 400x225 each, hence exactly 800x900.
TMP_FRAMES="$(mktemp -d)"
trap 'rm -rf "$TMP_FRAMES"' EXIT
TIMES=(0 3 6 9 12 15 18 20)
for i in "${!TIMES[@]}"; do
  ffmpeg -y -v error -ss "${TIMES[$i]}" -i "$MP4" -frames:v 1 \
    -vf "scale=400:225:flags=lanczos,drawbox=x=8:y=8:w=74:h=30:color=black@0.75:t=fill,drawtext=text='t=${TIMES[$i]}s':x=15:y=13:fontsize=18:fontcolor=white" \
    "$TMP_FRAMES/$(printf '%02d' "$i").png"
done
ffmpeg -y -v error -framerate 1 -i "$TMP_FRAMES/%02d.png" \
  -filter_complex 'tile=2x4:padding=0:margin=0' -frames:v 1 "$SHEET"

python3 - "$MANIFEST" "$RAW" "$MP4" "$WEBP" "$SHEET" "$EVIDENCE" "$WEBP_FPS" <<'PY'
import hashlib, json, subprocess, sys
from pathlib import Path
manifest, raw, mp4, webp, sheet, evidence, fps = sys.argv[1:]
def sha(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()
def probe(path):
    out = subprocess.check_output(["ffprobe", "-v", "error", "-show_entries",
        "format=duration:stream=width,height", "-of", "json", path], text=True)
    return json.loads(out)
artifacts = {}
for p in (raw, mp4, webp, sheet):
    item = {"path": p, "sha256": sha(p)}
    if p in (raw, mp4):
        item["probe"] = probe(p)
    artifacts[Path(p).name] = item
data = {
    "schema": 1, "published": False, "evidence_manifest": evidence,
    "evidence_sha256": sha(evidence), "webp_fps": int(fps),
    "artifacts": artifacts,
}
Path(manifest).write_text(json.dumps(data, indent=2) + "\n")
PY

"$HERE/validate-v2.sh" --dir "$OUT_DIR" --evidence "$EVIDENCE"
printf 'validated (not published):\n  %s\n  %s\n  %s\n  %s\n' "$MP4" "$WEBP" "$SHEET" "$MANIFEST"
