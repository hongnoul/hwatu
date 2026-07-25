#!/usr/bin/env bash
# Turn the uninterrupted real-product take into README media.
set -euo pipefail

RAW="${1:?usage: render-real.sh raw.mp4 [output-base]}"
BASE=${2:-"${RAW%.mp4}"}
MARKS="${RAW%.mp4}.marks"
[ -s "$RAW" ] || { echo "render-real: missing raw recording: $RAW" >&2; exit 1; }
[ -s "$MARKS" ] || { echo "render-real: missing beat markers: $MARKS" >&2; exit 1; }

# stage.sh deliberately gives wf-recorder 500 ms to initialize before the
# filmed clock begins. Remove that quiet lead-in, but otherwise keep this as a
# single continuous take with no cuts, overlays, captions, or speed changes.
START=0.52
END=$(awk '$2 == "end" { print $1 }' "$MARKS")
DURATION=$(awk -v end="$END" -v start=0.02 'BEGIN { printf "%.3f", end + start }')

ffmpeg -y -v error -ss "$START" -t "$DURATION" -i "$RAW" \
  -vf 'scale=1600:900:flags=lanczos' -c:v libx264 -crf 18 -preset slow \
  -pix_fmt yuv420p -movflags +faststart -an "$BASE.mp4"
ffmpeg -y -v error -ss "$START" -t "$DURATION" -i "$RAW" \
  -vf 'fps=15,scale=1100:619:flags=lanczos' -c:v libwebp -q:v 68 \
  -compression_level 5 -loop 0 -an "$BASE.webp"

# The contact sheet is an approval artifact, not part of the published media.
ffmpeg -y -v error -i "$BASE.mp4" \
  -vf 'fps=1/3,scale=800:450:flags=lanczos,tile=3x2:padding=8:margin=8' \
  -frames:v 1 "$BASE-contact-sheet.png"

printf 'mp4:     %s\nwebp:    %s\ncontact: %s\n' \
  "$BASE.mp4" "$BASE.webp" "$BASE-contact-sheet.png"
