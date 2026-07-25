#!/usr/bin/env bash
# One command to record, render, validate, and optionally publish the simple
# real-product README demo.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
OUT=/tmp/hwatu-demo-real
PUBLISH=0
while (($#)); do
  case "$1" in
    --out) OUT=${2:?--out requires a directory}; shift 2 ;;
    --publish) PUBLISH=1; shift ;;
    *) echo "usage: run-real.sh [--out DIR] [--publish]" >&2; exit 2 ;;
  esac
done
mkdir -p "$OUT"
RAW="$OUT/demo-real-raw.mp4"
BASE="$OUT/demo-v2"

"$HERE/film-real.sh" "$RAW"
"$HERE/render-real.sh" "$RAW" "$BASE"
"$HERE/validate-real.sh" "$BASE"
if ((PUBLISH)); then "$HERE/publish.sh" "$BASE"; fi
