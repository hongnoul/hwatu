#!/usr/bin/env bash
# Prepare, gate, record, render, and validate the AIUC README demo locally.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
RECORD="$HERE/../demo/record"
OUT=${HWATU_AIUC_DEMO_OUT:-/tmp/hwatu-demo-aiuc}
PUBLISH=0
while (($#)); do
  case "$1" in
    --out) OUT=${2:?--out requires a directory}; shift 2 ;;
    --publish) PUBLISH=1; shift ;;
    *) echo "usage: run.sh [--out DIR] [--publish]" >&2; exit 2 ;;
  esac
done

mkdir -p "$OUT"
"$HERE/prepare-fixtures.sh"
AIUC_EVIDENCE_DIR="$OUT/evidence" "$HERE/gate.sh"

RAW="$OUT/demo-aiuc-raw.mp4"
BASE="$OUT/demo-aiuc"
HWATU_DEMO_SCENARIO=aiuc \
HWATU_DEMO_REFERENCE_DIR="$HERE/reference" \
HWATU_DEMO_FINAL_DIR="$HERE/app" \
HWATU_DEMO_HWATU_BIN="${HWATU_DEMO_HWATU_BIN:-hwatu}" \
  "$RECORD/film-real.sh" "$RAW"
"$RECORD/render-real.sh" "$RAW" "$BASE"
"$RECORD/validate-real.sh" "$BASE"
cp "$HERE/evidence/fixture-manifest.json" "$OUT/evidence/fixture-manifest.json"

if ((PUBLISH)); then
  printf 'AIUC media passed local gates. Publishing changes public release assets and README.\n' >&2
  "$RECORD/publish.sh" "$BASE"
fi
