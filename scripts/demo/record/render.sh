#!/usr/bin/env bash
# render.sh — turn the raw film into publishable assets.
#
#   1. README hero: an animated webp loop (<15 s, autoplays on GitHub)
#   2. Release mp4: full quality, linked from the webp (jcode pattern)
#
# Usage:
#   scripts/demo/record/render.sh raw.mp4 [start] [duration]
#     start/duration select the README loop segment (default: the
#     climb+handoff beats read from raw.marks, else 0 +15s).
#
# Outputs next to the input: raw.readme.webp, raw.release.mp4
set -euo pipefail

RAW="${1:?usage: render.sh raw.mp4 [start] [dur]}"
MARKS="${RAW%.mp4}.marks"
START="${2:-}"
DUR="${3:-15}"

if [ -z "$START" ]; then
  # Default loop: begin at the first climb beat (or handoff, or 0).
  START=$(awk '/climb|handoff/{print $1; exit}' "$MARKS" 2>/dev/null || echo 0)
  START=${START:-0}
fi

BASE="${RAW%.mp4}"

# Release cut: just normalize container + faststart for web playback.
ffmpeg -y -v error -i "$RAW" -c:v libx264 -crf 18 -preset slow \
  -movflags +faststart -an "$BASE.release.mp4"

# README loop: 1280 wide, 20 fps animated webp, infinite loop.
ffmpeg -y -v error -ss "$START" -t "$DUR" -i "$RAW" \
  -vf "fps=20,scale=1280:-2:flags=lanczos" \
  -c:v libwebp -lossless 0 -q:v 70 -loop 0 -an "$BASE.readme.webp"

echo "release: $BASE.release.mp4 ($(du -h "$BASE.release.mp4" | cut -f1))"
echo "readme:  $BASE.readme.webp ($(du -h "$BASE.readme.webp" | cut -f1))"
echo
echo "Publish (jcode pattern):"
echo "  gh release upload readme-assets $BASE.release.mp4 $BASE.readme.webp"
echo '  README: <a href="...release.mp4"><img src="...readme.webp"></a>'
