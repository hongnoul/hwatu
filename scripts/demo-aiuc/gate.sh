#!/usr/bin/env bash
# Verify the AIUC reference/app pair across the viewport matrix used on film.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
HWATU=${HWATU_BIN:-hwatu}
REF_PORT=${AIUC_REF_PORT:-8421}
APP_PORT=${AIUC_APP_PORT:-8422}
OUT=${AIUC_EVIDENCE_DIR:-"$HERE/evidence"}
MIN_MATCH=${AIUC_MIN_MATCH:-99.0}
SETTLE_SECONDS=${AIUC_SETTLE_SECONDS:-2}
PIDS=()

cleanup() {
  for id in "${REF_ID:-}" "${APP_ID:-}"; do
    [ -n "$id" ] && "$HWATU" close "$id" >/dev/null 2>&1 || true
  done
  for pid in "${PIDS[@]:-}"; do kill "$pid" >/dev/null 2>&1 || true; done
}
trap cleanup EXIT HUP INT TERM

[ -r "$HERE/reference/index.html" ] || "$HERE/prepare-fixtures.sh"
mkdir -p "$OUT"
python3 -m http.server "$REF_PORT" --bind 127.0.0.1 --directory "$HERE/reference" >/dev/null 2>&1 & PIDS+=("$!")
python3 -m http.server "$APP_PORT" --bind 127.0.0.1 --directory "$HERE/app" >/dev/null 2>&1 & PIDS+=("$!")
sleep 0.4

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
REF_ID=$(open_id "http://127.0.0.1:$REF_PORT/")
APP_ID=$(open_id "http://127.0.0.1:$APP_PORT/")
"$HWATU" wait-load --id "$REF_ID" --timeout-ms 30000 >/dev/null
"$HWATU" wait-load --id "$APP_ID" --timeout-ms 30000 >/dev/null

: >"$OUT/viewport-diffs.jsonl"
for size in 390x844 768x1024 1440x900 1920x1080; do
  "$HWATU" resize --id "$REF_ID" "$size" >/dev/null
  "$HWATU" resize --id "$APP_ID" "$size" >/dev/null
  sleep "$SETTLE_SECONDS"
  result=
  for attempt in 1 2 3; do
    if result=$("$HWATU" diff --id "$APP_ID" --other "$REF_ID"); then break; fi
    [ "$attempt" = 3 ] && exit 1
    sleep "$SETTLE_SECONDS"
  done
  python3 - "$size" "$MIN_MATCH" "$result" >>"$OUT/viewport-diffs.jsonl" <<'PY'
import json, sys
size, minimum, raw = sys.argv[1:]
data = json.loads(raw)
data["requested_viewport"] = size
data["minimum_match_percent"] = float(minimum)
if data["match_percent"] < float(minimum):
    raise SystemExit(f"viewport gate failed at {size}: {data['match_percent']}% < {minimum}%")
print(json.dumps(data, sort_keys=True))
PY
done
python3 - "$OUT/viewport-diffs.jsonl" <<'PY'
import json, pathlib, sys
rows = [json.loads(line) for line in pathlib.Path(sys.argv[1]).read_text().splitlines()]
print(json.dumps({"status": "PASS", "minimum_match_percent": rows[0]["minimum_match_percent"],
                  "viewports": [r["requested_viewport"] for r in rows],
                  "scores": [r["match_percent"] for r in rows]}, indent=2))
PY
