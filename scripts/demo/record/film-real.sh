#!/usr/bin/env bash
# Record one uninterrupted, real hwatu workflow inside the isolated stage.
# There are no presentation layers: every visible command and browser frame is
# produced by the released CLI and WebKit daemon.
set -euo pipefail

OUT="${1:?usage: film-real.sh out.mp4}"
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
DEMO_DIR=$(cd "$HERE/.." && pwd)
STAGE="$HERE/stage.sh"
REF_DIR=${HWATU_DEMO_REFERENCE_DIR:-"$DEMO_DIR/reference"}
APP_DIR=${HWATU_DEMO_FINAL_DIR:-"$DEMO_DIR/checkpoints/07-final-99.84pct"}
REF_PORT=${HWATU_DEMO_REF_PORT:-8321}
APP_PORT=${HWATU_DEMO_APP_PORT:-8322}
MARKS="${OUT%.mp4}.marks"
PIDS=()

fail() { printf 'film-real: %s\n' "$*" >&2; exit 1; }
pick_port() { python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'; }

[ -r "$REF_DIR/index.html" ] || fail "missing reference fixture: $REF_DIR"
[ -r "$APP_DIR/index.html" ] || fail "missing app fixture: $APP_DIR"
mkdir -p "$(dirname "$OUT")"
OUT=$(cd "$(dirname "$OUT")" && pwd)/$(basename "$OUT")
rm -f "$OUT" "$MARKS"

# Avoid colliding with another local demo run while keeping the URLs short in
# the filmed terminal. Explicit port overrides remain available for debugging.
if ! python3 - "$REF_PORT" <<'PY' >/dev/null 2>&1
import socket, sys
s=socket.socket(); s.bind(('127.0.0.1', int(sys.argv[1]))); s.close()
PY
then REF_PORT=$(pick_port); fi
if ! python3 - "$APP_PORT" <<'PY' >/dev/null 2>&1
import socket, sys
s=socket.socket(); s.bind(('127.0.0.1', int(sys.argv[1]))); s.close()
PY
then APP_PORT=$(pick_port); fi

cleanup() {
  "$STAGE" down >/dev/null 2>&1 || true
  for pid in "${PIDS[@]:-}"; do kill "$pid" >/dev/null 2>&1 || true; done
  for pid in "${PIDS[@]:-}"; do wait "$pid" >/dev/null 2>&1 || true; done
}
trap cleanup EXIT HUP INT TERM

python3 -m http.server "$REF_PORT" --bind 127.0.0.1 --directory "$REF_DIR" >/dev/null 2>&1 & PIDS+=("$!")
python3 -m http.server "$APP_PORT" --bind 127.0.0.1 --directory "$APP_DIR" >/dev/null 2>&1 & PIDS+=("$!")
for _ in $(seq 1 100); do
  curl -fsS -o /dev/null "http://127.0.0.1:$REF_PORT/" 2>/dev/null \
    && curl -fsS -o /dev/null "http://127.0.0.1:$APP_PORT/" 2>/dev/null && break
  sleep 0.05
done
curl -fsS -o /dev/null "http://127.0.0.1:$APP_PORT/" || fail "fixture servers did not start"

export HWATU_DEMO_TYPE_DELAY=${HWATU_DEMO_TYPE_DELAY:-0.018}
"$STAGE" up

# Prepare a plain shell before rolling. The environment variables make the
# commands readable at README width without hiding what they do.
old_delay=$HWATU_DEMO_TYPE_DELAY
export HWATU_DEMO_TYPE_DELAY=0
"$STAGE" type "export PS1='$ '; REF=http://127.0.0.1:$REF_PORT/; APP=http://127.0.0.1:$APP_PORT/; clear"
export HWATU_DEMO_TYPE_DELAY=$old_delay
sleep 0.4

"$STAGE" rec "$OUT"
T0=$(date +%s.%N)
mark() {
  local now
  now=$(date +%s.%N)
  awk -v now="$now" -v start="$T0" -v label="$1" \
    'BEGIN { printf "%.3f %s\n", now - start, label }' >> "$MARKS"
}
run() { mark "$1"; "$STAGE" type "$2"; sleep "$3"; }

run open-reference 'hwatu --headless --json "$REF" | jq '\''{id,mode}'\''' 1.0
run open-app 'hwatu --headless --json "$APP" | jq '\''{id,mode}'\''' 1.0
run wait 'hwatu wait-load --id 1; hwatu wait-load --id 2' 0.7
run verify 'hwatu diff --id 2 --other 1 | jq '\''{match_percent}'\''' 2.0
run handoff 'hwatu focus 2' 1.5
run scroll-down 'hwatu scroll --id 2 --to-y 500' 1.2
run scroll-home 'hwatu scroll --id 2 --to-y 0' 2.0

mark end
"$STAGE" stoprec
trap - EXIT HUP INT TERM
cleanup
printf 'raw film: %s\nmarkers:  %s\n' "$OUT" "$MARKS"
