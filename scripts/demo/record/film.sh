#!/usr/bin/env bash
# film.sh — the automated demo shoot, end to end.
#
# Runs the whole convergence-demo recording inside the invisible
# stage: no human at the keyboard, no window on the real desktop.
# The result is a raw mp4 (plus per-beat marker file) ready for
# cutting; render.sh turns it into the README webp + release mp4.
#
# Prereqs:
#   - scripts/demo/ fixtures ready (reference mirror + clone states)
#   - `hwatu` in PATH (stage uses its own isolated daemon)
#
# Usage:
#   scripts/demo/record/film.sh out/demo-raw.mp4
#
# Beat markers: every `mark <label>` appends "<t_seconds> <label>" to
# out/demo-raw.marks so the cut points are machine-readable.
set -euo pipefail

OUT="${1:?usage: film.sh out.mp4}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEMO_DIR="$(dirname "$HERE")"
STAGE="$HERE/stage.sh"
MARKS="${OUT%.mp4}.marks"
mkdir -p "$(dirname "$OUT")"

REF_PORT=8321
CLONE_PORT=8322

T0=
mark() {
  local now; now=$(date +%s.%N)
  awk -v now="$now" -v start="$T0" -v label="$1" \
    'BEGIN { printf "%.3f %s\n", now - start, label }' >> "$MARKS"
}

say() { "$STAGE" type "$1"; }
pause() { sleep "$1"; }

cleanup() {
  "$STAGE" down || true
  kill "${REF_SRV:-}" "${CLONE_SRV:-}" 2>/dev/null || true
}
trap cleanup EXIT

# ---- fixtures ----------------------------------------------------
[ -d "$DEMO_DIR/reference" ] || { echo "run $DEMO_DIR/fetch-reference.sh first"; exit 1; }
CLONE_DIR="${HWATU_DEMO_CLONE_DIR:-$DEMO_DIR/clone2}"
[ -d "$CLONE_DIR" ] || CLONE_DIR="$DEMO_DIR/clone"
python3 -m http.server $REF_PORT --directory "$DEMO_DIR/reference" >/dev/null 2>&1 &
REF_SRV=$!
python3 -m http.server $CLONE_PORT --directory "$CLONE_DIR" >/dev/null 2>&1 &
CLONE_SRV=$!

# ---- stage up + roll ---------------------------------------------
"$STAGE" up
rm -f "$MARKS"
"$STAGE" rec "$OUT"
T0=$(date +%s.%N)

# Beat 1: cold open — two headless sessions + first diff.
mark cold-open
say "hwatu --headless --json http://localhost:$REF_PORT/"
pause 2
say "hwatu --headless --json http://localhost:$CLONE_PORT/"
pause 2
say "hwatu wait-load --id 1 --timeout-ms 20000 && hwatu wait-load --id 2"
pause 4
say "hwatu diff --id 2 --other 1 --heatmap /tmp/heat.png"
pause 3

# Beat 2: read the reference's motion as numbers.
mark motion
say "hwatu motion --id 1"
pause 4

# Beat 3: pin animations mid-flight, byte-comparable shots.
mark seek
say "hwatu seek --id 1 --progress 0.5 && hwatu shot --id 1 /tmp/ref-mid.png"
pause 2
say "hwatu seek --id 2 --progress 0.5 && hwatu shot --id 2 /tmp/clone-mid.png"
pause 2
say "hwatu seek --id 1 --resume && hwatu seek --id 2 --resume"
pause 2

# Beat 4 (optional, if checkpoint dirs exist): the climb.
# Each checkpoint dir is a progressively better clone; re-serve + re-diff.
# IMPORTANT: diff at CLIMB_SCROLL, not page top. Validated 2026-07-24:
# at scroll 0 every checkpoint scores ~93% (hero was cloned first) and
# the climb is invisible; at 75% it runs 0.4% -> 98%. Tune per take.
CLIMB_SCROLL="${HWATU_DEMO_CLIMB_SCROLL:-75}"
SCROLL_JS="window.scrollTo(0,(document.documentElement.scrollHeight-innerHeight)*$CLIMB_SCROLL/100)"
for ckpt in "$DEMO_DIR"/checkpoints/*/; do
  [ -d "$ckpt" ] || continue
  mark "climb $(basename "$ckpt")"
  kill $CLONE_SRV 2>/dev/null || true
  python3 -m http.server $CLONE_PORT --directory "$ckpt" >/dev/null 2>&1 &
  CLONE_SRV=$!
  pause 1
  # A distinct URL defeats WebKit's HTTP cache after the server swaps
  # the files behind this port. Without it, every checkpoint can render
  # the first checkpoint even though the server directory changed.
  say "hwatu goto --id 2 http://localhost:$CLONE_PORT/?checkpoint=$(basename "$ckpt")"
  pause 3
  say "hwatu eval --id 1 '$SCROLL_JS' >/dev/null; hwatu eval --id 2 '$SCROLL_JS' >/dev/null"
  pause 1
  say "hwatu diff --id 2 --other 1"
  pause 3
done

# Beat 5: the hand-off. Both sessions materialize in the tiler.
mark handoff
say "hwatu focus 1 && hwatu focus 2"
pause 5

# Beat 6: close card material — synchronized scroll.
mark close
say "hwatu scroll --id 1 --to-y 600; hwatu scroll --id 2 --to-y 600"
pause 4

"$STAGE" stoprec
echo "raw film: $OUT"
echo "markers:  $MARKS"
