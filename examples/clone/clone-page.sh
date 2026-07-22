#!/usr/bin/env bash
# clone-page.sh — capture a live page from a hwatu window into a
# self-contained local mirror, then verify with `hwatu diff`.
#
# The capture strategy: serialize the *rendered* DOM (post-JS), inline
# every same-and-cross-origin stylesheet the page actually loaded
# (via CSSOM, so CORS-blocked sheets fall back to href fetch), record
# canvas/video poster frames as data URLs, and list every external
# asset (img/src, srcset, fonts, background urls) for mirroring.
set -euo pipefail

TESTDIR="${TESTDIR:?set TESTDIR}"
H() { XDG_RUNTIME_DIR="$TESTDIR/runtime" ~/git/hwatu/target/release/hwatu "$@"; }
OUT="${1:?usage: clone-page.sh <outdir> [window-id]}"
ID="${2:-1}"
mkdir -p "$OUT/assets"

# Phase 1: extract rendered HTML + asset manifest from the live page.
H eval --id "$ID" --timeout-ms 60000 "$(cat "$(dirname "$0")/extract.js")" > "$OUT/capture.json"
python3 "$(dirname "$0")/materialize.py" "$OUT"
echo "clone written to $OUT"
