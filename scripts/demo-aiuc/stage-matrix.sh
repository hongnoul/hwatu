#!/usr/bin/env bash
# Run the exact responsive verification sequence inside the isolated demo stage.
set -euo pipefail

APP=${1:?usage: stage-matrix.sh APP_URL REF_URL}
REF=${2:?usage: stage-matrix.sh APP_URL REF_URL}
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
HWATU="$HERE/../demo/record/stage-hwatu.sh"
SETTLE_SECONDS=${AIUC_SETTLE_SECONDS:-2}
MIN_MATCH=${AIUC_MIN_MATCH:-99.0}
COMPLETE_FILE=${AIUC_DEMO_COMPLETE_FILE:-}
[ -z "$COMPLETE_FILE" ] || trap ': >"$COMPLETE_FILE"' EXIT

open_id() {
  local url=$1 raw
  for attempt in 1 2 3; do
    if raw=$("$HWATU" --headless --json "$url"); then
      python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])' <<<"$raw"
      return
    fi
    [ "$attempt" = 3 ] && return 1
    sleep "$SETTLE_SECONDS"
  done
}

APP_ID=$(open_id "$APP")
"$HWATU" wait-load --id "$APP_ID" --timeout-ms 30000 >/dev/null
# Put the actual product on screen before doing any background measurement.
# The recording should answer "what is Hwatu showing me?" immediately rather
# than making the viewer watch a verification matrix before the reveal.
"$HWATU" focus "$APP_ID" >/dev/null

REF_ID=$(open_id "$REF")
"$HWATU" wait-load --id "$REF_ID" --timeout-ms 30000 >/dev/null

printf 'AIUC responsive verification\n'
printf 'app=%s ref=%s\n' "$APP_ID" "$REF_ID"
for size in 390x844 768x1024 1440x900 1920x1080; do
  "$HWATU" resize --id "$APP_ID" "$size" >/dev/null
  "$HWATU" resize --id "$REF_ID" "$size" >/dev/null
  sleep "$SETTLE_SECONDS"
  result=
  for attempt in 1 2 3; do
    if result=$("$HWATU" diff --id "$APP_ID" --other "$REF_ID"); then break; fi
    [ "$attempt" = 3 ] && exit 1
    sleep "$SETTLE_SECONDS"
  done
  python3 - "$size" "$MIN_MATCH" "$result" <<'PY'
import json, sys
size, minimum, raw = sys.argv[1:]
data = json.loads(raw)
print(f'{size}: {data["match_percent"]:.2f}% '
      f'({data["mismatched_pixels"]} mismatched pixels)')
if data["match_percent"] < float(minimum):
    raise SystemExit(f'FAIL: {size} fell below the {minimum}% recording gate')
PY
done
printf 'Caveat: each score covers this WebKitGTK engine, viewport, and frame only.\n'
"$HWATU" focus "$APP_ID" >/dev/null
printf 'PASS: focused the same live app session for human review.\n'
