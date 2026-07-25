#!/usr/bin/env bash
# Fail-closed local publish gate for killer-demo v2. It never uploads.
#
# Usage:
#   scripts/demo/record/validate-v2.sh --dir /tmp/hwatu-demo-v2 \
#     --evidence /path/to/evidence-manifest.json
set -euo pipefail

DIR=""
EVIDENCE=""
die() { printf 'validate-v2: FAIL: %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"; }
while (($#)); do
  case "$1" in
    --dir) DIR="${2:?--dir requires a value}"; shift 2 ;;
    --evidence) EVIDENCE="${2:?--evidence requires a value}"; shift 2 ;;
    -h|--help) sed -n '2,7p' "$0"; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done
[[ -n "$DIR" && -d "$DIR" ]] || die "--dir must be an output directory"
[[ -n "$EVIDENCE" && -s "$EVIDENCE" ]] || die "captured evidence manifest is missing or empty"
need ffprobe
need ffmpeg
need python3
python3 -c 'from PIL import Image' 2>/dev/null || die "Pillow is required"

DIR="$(cd "$DIR" && pwd)"
EVIDENCE="$(realpath "$EVIDENCE")"
RAW="$DIR/demo-v2.raw.mp4"
MP4="$DIR/demo-v2.mp4"
WEBP="$DIR/demo-v2.webp"
SHEET="$DIR/demo-v2-contact-sheet.png"
MANIFEST="$DIR/demo-v2-render-manifest.json"
for path in "$RAW" "$MP4" "$WEBP" "$SHEET" "$MANIFEST"; do
  [[ -s "$path" ]] || die "artifact missing or empty: $path"
done

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
# Ten samples/second make the 700 ms idle limit directly observable.
ffmpeg -v error -i "$MP4" -vf 'fps=10,scale=320:180:flags=area' "$TMP/frame-%04d.png"

python3 - "$RAW" "$MP4" "$WEBP" "$SHEET" "$MANIFEST" "$EVIDENCE" "$TMP" <<'PY'
import hashlib, json, math, statistics, subprocess, sys
from pathlib import Path
from PIL import Image, ImageChops, ImageFilter, ImageStat
raw, mp4, webp, sheet, manifest_path, evidence, frames_dir = map(Path, sys.argv[1:])

def fail(message):
    raise SystemExit("validate-v2: FAIL: " + message)
def sha(path):
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()
def probe(path):
    try:
        data = json.loads(subprocess.check_output([
            "ffprobe", "-v", "error", "-select_streams", "v:0",
            "-show_entries", "stream=width,height:format=duration", "-of", "json", str(path)
        ], text=True))
        stream = data["streams"][0]
        return int(stream["width"]), int(stream["height"]), float(data["format"]["duration"])
    except Exception as exc:
        fail(f"ffprobe could not decode {path.name}: {exc}")

def check_video(path, dimensions):
    width, height, duration = probe(path)
    if (width, height) != dimensions:
        fail(f"{path.name} is {width}x{height}, expected {dimensions[0]}x{dimensions[1]}")
    if not 20.0 <= duration <= 22.0:
        fail(f"{path.name} duration {duration:.3f}s is outside 20-22s")
    return duration

raw_duration = check_video(raw, (1600, 900))
mp4_duration = check_video(mp4, (1280, 720))
# The raw provenance capture includes wf-recorder's deliberate ~500 ms
# paused-frame preroll. render-v2 removes it from the published cut.
if abs(raw_duration - mp4_duration) > 0.75:
    fail("raw and MP4 durations differ by more than the 750 ms preroll allowance")

try:
    with Image.open(webp) as image:
        if image.size != (1280, 720) or not getattr(image, "is_animated", False):
            fail("WebP must be animated at exactly 1280x720")
        webp_frames = image.n_frames
        # seek/load every frame, not merely container metadata
        for index in range(webp_frames):
            image.seek(index)
            image.convert("RGB").load()
        webp_duration = sum((image.seek(i), image.info.get("duration", 0))[1]
                            for i in range(webp_frames)) / 1000
except SystemExit:
    raise
except Exception as exc:
    fail(f"Pillow could not fully decode WebP: {exc}")

try:
    manifest = json.loads(manifest_path.read_text())
except Exception as exc:
    fail(f"render manifest is invalid JSON: {exc}")
if manifest.get("published") is not False:
    fail("render manifest must explicitly state published=false")
if Path(manifest.get("evidence_manifest", "")).resolve() != evidence.resolve():
    fail("render manifest does not identify the supplied evidence manifest")
if manifest.get("evidence_sha256") != sha(evidence):
    fail("captured evidence manifest hash changed")
fps = manifest.get("webp_fps")
if not isinstance(fps, int) or fps <= 0:
    fail("render manifest has invalid webp_fps")
expected_frames = round(mp4_duration * fps)
if abs(webp_frames - expected_frames) > 1:
    fail(f"WebP has {webp_frames} frames, expected {expected_frames}±1")
if not 20.0 <= webp_duration <= 22.1:
    fail(f"WebP decoded duration {webp_duration:.3f}s is outside tolerance")
for name, item in manifest.get("artifacts", {}).items():
    path = Path(item.get("path", ""))
    if not path.is_file() or item.get("sha256") != sha(path):
        fail(f"manifest hash mismatch for {name}")

with Image.open(sheet) as image:
    image.load()
    if image.width != 800:
        fail(f"contact sheet width is {image.width}, expected 800")
    if image.height != 900:
        fail(f"contact sheet height is {image.height}, expected 900 (2x4 timestamp grid)")

frames = [Image.open(p).convert("L") for p in sorted(frames_dir.glob("frame-*.png"))]
if len(frames) < 200:
    fail(f"only {len(frames)} 10fps MP4 samples decoded, expected at least 200")
# Pixel activity threshold is deliberately low enough to catch rail/counter motion,
# but high enough to reject encoder shimmer. No run may be idle for >700 ms.
diffs = [ImageStat.Stat(ImageChops.difference(a, b)).mean[0] for a, b in zip(frames, frames[1:])]
active = [value >= 0.12 for value in diffs]
longest = run = 0
for changed in active:
    run = 0 if changed else run + 1
    longest = max(longest, run)
if longest > 7:
    fail(f"longest idle span is {longest / 10:.1f}s, limit is 0.7s")
if sum(active) < len(active) * 0.35:
    fail(f"only {sum(active)}/{len(active)} sampled transitions visibly differ")

# The intended five beats are 2/7/6/4/1 seconds. Require visible activity in
# each beat so a frozen/missing scene cannot pass on aggregate motion alone.
beats = [(0, 2, "open"), (2, 9, "measure"), (9, 15, "pin-motion"),
         (15, 19, "hand-off"), (19, 20, "end")]
for start, end, label in beats:
    section = active[start * 10:min(end * 10, len(active))]
    if not section or sum(section) < max(1, len(section) // 5):
        fail(f"{label} beat lacks visible activity in its {end-start}s window")

# Readability proxy at delivered size: all scene samples must have enough
# high-contrast edge detail in both the label zone and 28px rail zone.
# This cannot recognize words, but rejects blank, tiny, or washed-out labels.
for second, label in ((3, "MEASURE"), (10, "PIN MOTION"), (16, "HAND OFF")):
    frame = frames[min(second * 10, len(frames) - 1)]
    for zone_name, box in (("label", (0, 0, 320, 50)), ("rail", (0, 145, 320, 180))):
        zone = frame.crop(box)
        edges = zone.filter(ImageFilter.FIND_EDGES)
        edge_ratio = sum(edges.histogram()[32:]) / (zone.width * zone.height)
        contrast = ImageStat.Stat(zone).stddev[0]
        if edge_ratio < 0.012 or contrast < 4.5:
            fail(f"{label} {zone_name} readability proxy failed: edge={edge_ratio:.4f}, contrast={contrast:.1f}")

# Loop proxy. A gentle transition is allowed, but a hard unrelated cut is not.
loop_delta = ImageStat.Stat(ImageChops.difference(frames[0], frames[-1])).mean[0]
if loop_delta > 45:
    fail(f"first/last frame delta {loop_delta:.1f} is too large for a seamless loop")

print(json.dumps({
    "status": "PASS", "raw_duration": round(raw_duration, 3),
    "mp4_duration": round(mp4_duration, 3), "webp_frames": webp_frames,
    "webp_duration": round(webp_duration, 3), "contact_sheet": "800x900",
    "active_transitions": f"{sum(active)}/{len(active)}",
    "longest_idle_seconds": longest / 10, "loop_delta": round(loop_delta, 2),
    "evidence_sha256": sha(evidence), "published": False,
}, indent=2))
PY
