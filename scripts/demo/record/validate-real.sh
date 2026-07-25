#!/usr/bin/env bash
# Mechanical checks for the direct product recording. Semantic approval is
# done from the generated 800 px contact sheet.
set -euo pipefail

BASE="${1:?usage: validate-real.sh output-base}"
MP4="$BASE.mp4"
WEBP="$BASE.webp"
SHEET="$BASE-contact-sheet.png"
for file in "$MP4" "$WEBP" "$SHEET"; do [ -s "$file" ] || { echo "validate-real: missing $file" >&2; exit 1; }; done

python3 - "$MP4" "$WEBP" "$SHEET" <<'PY'
import json, subprocess, sys
from PIL import Image, ImageSequence

mp4, webp, sheet = sys.argv[1:]
probe = json.loads(subprocess.check_output([
    'ffprobe', '-v', 'error', '-show_entries',
    'format=duration:stream=codec_name,width,height', '-of', 'json', mp4
]))
stream = probe['streams'][0]
duration = float(probe['format']['duration'])
if stream['codec_name'] != 'h264' or (stream['width'], stream['height']) != (1600, 900):
    raise SystemExit(f"unexpected MP4 stream: {stream}")
if not 12 <= duration <= 45:
    raise SystemExit(f"unexpected duration: {duration:.3f}s")

with Image.open(webp) as image:
    if image.size != (1100, 619) or not getattr(image, 'is_animated', False):
        raise SystemExit(f"unexpected WebP: size={image.size}, animated={getattr(image, 'is_animated', False)}")
    frames = sum(1 for _ in ImageSequence.Iterator(image))
    if frames < 100:
        raise SystemExit(f"too few WebP frames: {frames}")
with Image.open(sheet) as image:
    if image.size != (2432, 924):
        raise SystemExit(f"unexpected contact sheet: {image.size}")

print(json.dumps({
    'status': 'PASS', 'duration': round(duration, 3),
    'mp4': [stream['width'], stream['height']], 'webp_frames': frames,
    'contact_sheet': [2432, 924], 'presentation_layers': 0,
}, indent=2))
PY
