#!/usr/bin/env bash
# Capture, compose, validate, and optionally publish the visual-first README demo.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEMO_DIR="$(dirname "$HERE")"
REPO_ROOT="$(cd "$DEMO_DIR/../.." && pwd)"
PUBLISH=false
OUT="${HWATU_DEMO_V2_OUT:-/tmp/hwatu-demo-v2}"

while (($#)); do
  case "$1" in
    --publish) PUBLISH=true; shift ;;
    --out) OUT="${2:?--out requires a directory}"; shift 2 ;;
    -h|--help)
      echo "usage: $0 [--out DIR] [--publish]"
      exit 0
      ;;
    *) echo "run-v2: unknown argument: $1" >&2; exit 1 ;;
  esac
done

BOOKEND_DIR="${HWATU_DEMO_BOOKEND_DIR:-$DEMO_DIR/checkpoints/05-integrated-97pct}"
FINAL_DIR="${HWATU_DEMO_FINAL_DIR:-$DEMO_DIR/checkpoints/07-final-99.84pct}"
BOOKEND_SCORECARD="${HWATU_DEMO_BOOKEND_SCORECARD:-$DEMO_DIR/scorecards/05-integrated.json}"
FINAL_SCORECARD="${HWATU_DEMO_FINAL_SCORECARD:-$DEMO_DIR/scorecards/07-final.json}"
EVIDENCE="$OUT/evidence"
RENDER="$OUT/render"
SERVER_LOG="$OUT/server.log"
mkdir -p "$OUT"

for path in "$BOOKEND_DIR/index.html" "$FINAL_DIR/index.html" "$BOOKEND_SCORECARD" "$FINAL_SCORECARD"; do
  [ -r "$path" ] || { echo "run-v2: required fixture missing: $path" >&2; exit 1; }
done

if [ ! -x "$REPO_ROOT/target/release/hwatu" ]; then
  echo "JCODE_CHECKPOINT {\"message\":\"Building release binary\"}"
  cargo build --release --manifest-path "$REPO_ROOT/Cargo.toml"
fi

echo "JCODE_PROGRESS {\"current\":1,\"total\":3,\"unit\":\"phases\",\"message\":\"Capturing real WebKit evidence\"}"
HWATU_DEMO_BOOKEND_DIR="$BOOKEND_DIR" \
HWATU_DEMO_FINAL_DIR="$FINAL_DIR" \
HWATU_DEMO_BOOKEND_SCORECARD="$BOOKEND_SCORECARD" \
HWATU_DEMO_FINAL_SCORECARD="$FINAL_SCORECARD" \
  "$HERE/capture-v2.sh" "$EVIDENCE"

PORT="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')"
python3 -m http.server "$PORT" --bind 127.0.0.1 --directory / >"$SERVER_LOG" 2>&1 &
SERVER_PID=$!
cleanup() { kill "$SERVER_PID" >/dev/null 2>&1 || true; wait "$SERVER_PID" >/dev/null 2>&1 || true; }
trap cleanup EXIT INT TERM
for _ in $(seq 1 100); do
  curl -fsS -o /dev/null "http://127.0.0.1:$PORT/" && break
  sleep 0.05
done
curl -fsS -o /dev/null "http://127.0.0.1:$PORT/" || { echo "run-v2: local asset server failed" >&2; exit 1; }

COMPOSE_PATH="${HERE#/}/compose-v2.html"
EVIDENCE_PATH="${EVIDENCE#/}"
URL="http://127.0.0.1:$PORT/$COMPOSE_PATH?evidence=/$EVIDENCE_PATH"
echo "JCODE_PROGRESS {\"current\":2,\"total\":3,\"unit\":\"phases\",\"message\":\"Recording and validating the 21-second story\"}"
rm -rf "$RENDER"
"$HERE/render-v2.sh" --url "$URL" --evidence "$EVIDENCE/evidence-manifest.json" --out-dir "$RENDER"

echo "JCODE_PROGRESS {\"current\":3,\"total\":3,\"unit\":\"phases\",\"message\":\"Validated outputs ready\"}"
if [ "$PUBLISH" = true ]; then
  "$HERE/publish.sh" "$RENDER/demo-v2"
else
  printf 'validated, not published:\n  %s\n  %s\n  %s\n' \
    "$RENDER/demo-v2.mp4" "$RENDER/demo-v2.webp" "$RENDER/demo-v2-contact-sheet.png"
  echo "publish after review: $0 --out '$OUT' --publish"
fi
