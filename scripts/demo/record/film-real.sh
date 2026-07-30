#!/usr/bin/env bash
# Record one uninterrupted, real Jcode -> hwatu verification workflow inside
# the isolated stage. Every visible frame is the actual Jcode TUI or the live
# WebKit session it hands to the human. There are no presentation layers.
set -euo pipefail

OUT="${1:?usage: film-real.sh out.mp4}"
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO_DIR=$(cd "$HERE/../../.." && pwd)
DEMO_DIR=$(cd "$HERE/.." && pwd)
STAGE="$HERE/stage.sh"
STAGE_HWATU="$HERE/stage-hwatu.sh"
REF_DIR=${HWATU_DEMO_REFERENCE_DIR:-"$DEMO_DIR/reference"}
APP_DIR=${HWATU_DEMO_FINAL_DIR:-"$DEMO_DIR/checkpoints/07-final-99.84pct"}
REF_PORT=${HWATU_DEMO_REF_PORT:-8321}
APP_PORT=${HWATU_DEMO_APP_PORT:-8322}
JCODE_BIN=${HWATU_DEMO_JCODE_BIN:-$(command -v jcode || true)}
JCODE_MODEL=${HWATU_DEMO_JCODE_MODEL:-gpt-5.5}
MARKS="${OUT%.mp4}.marks"
PIDS=()

fail() { printf 'film-real: %s\n' "$*" >&2; exit 1; }
pick_port() { python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'; }

[ -x "$JCODE_BIN" ] || fail "jcode is not installed"
[ -x "$STAGE_HWATU" ] || fail "stage-hwatu.sh is not executable"
[ -r "$REF_DIR/index.html" ] || fail "missing reference fixture: $REF_DIR"
[ -r "$APP_DIR/index.html" ] || fail "missing app fixture: $APP_DIR"
mkdir -p "$(dirname "$OUT")"
OUT=$(cd "$(dirname "$OUT")" && pwd)/$(basename "$OUT")
JCODE_DIR="${OUT%.mp4}-jcode"
JCODE_SOCKET="$JCODE_DIR/jcode.sock"
JCODE_LOG="$JCODE_DIR/server.log"
COMPLETE_FILE="$JCODE_DIR/verification.complete"
rm -rf "$JCODE_DIR"
mkdir -p -m 700 "$JCODE_DIR/run"
rm -f "$OUT" "$MARKS"

# Avoid colliding with another local demo run while keeping the URLs short in
# the filmed prompt. Explicit port overrides remain available for debugging.
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

# Use a separate runtime lock/socket, but keep the user's authenticated Jcode
# provider configuration. Tool exposure is deliberately reduced to bash for a
# small, legible, honest verification session.
XDG_RUNTIME_DIR="$JCODE_DIR/run" JCODE_NO_TELEMETRY=1 \
  AIUC_DEMO_COMPLETE_FILE="$COMPLETE_FILE" \
  "$JCODE_BIN" serve --socket "$JCODE_SOCKET" --server-name Demo \
  --provider jcode --model "$JCODE_MODEL" --tool-profile none --tools bash \
  --no-update >"$JCODE_LOG" 2>&1 & PIDS+=("$!")
for _ in $(seq 1 200); do
  [ -S "$JCODE_SOCKET" ] && break
  kill -0 "${PIDS[-1]}" 2>/dev/null || { cat "$JCODE_LOG" >&2; fail "Jcode server exited"; }
  sleep 0.05
done
[ -S "$JCODE_SOCKET" ] || { cat "$JCODE_LOG" >&2; fail "Jcode server did not create its socket"; }
grep -q "Using model: $JCODE_MODEL" "$JCODE_LOG" \
  || { cat "$JCODE_LOG" >&2; fail "Jcode did not select $JCODE_MODEL"; }

export HWATU_DEMO_TYPE_DELAY=${HWATU_DEMO_TYPE_DELAY:-0.006}
"$STAGE" up

# Start hwatud with the complete isolated XDG environment before filming. A
# bare ping creates no page, and the empty-list assertion prevents accidental
# exposure of restored personal sessions.
"$STAGE_HWATU" ping >/dev/null
[ "$("$STAGE_HWATU" list --json)" = "[]" ] || fail "isolated hwatud restored unexpected sessions"

# Enter the real TUI before rolling so the film begins directly on the product.
old_delay=$HWATU_DEMO_TYPE_DELAY
export HWATU_DEMO_TYPE_DELAY=0
"$STAGE" type "clear; jcode --socket '$JCODE_SOCKET' --remote-working-dir '$REPO_DIR' --no-update"
export HWATU_DEMO_TYPE_DELAY=$old_delay
sleep 2.5

REF="http://127.0.0.1:$REF_PORT/"
APP="http://127.0.0.1:$APP_PORT/"
if [ "${HWATU_DEMO_SCENARIO:-stripe}" = aiuc ]; then
  PROMPT="Verify this AIUC preview. Run only: scripts/demo-aiuc/stage-matrix.sh '$APP' '$REF'. Do not inspect or edit files. Return only the four viewport scores and the caveat."
else
  PROMPT="Compare APP $APP with REF $REF using scripts/demo/record/stage-hwatu.sh. Open both headless, wait, diff, report only the score, then focus APP. Do not edit files."
fi
# The film opens with the full prompt already sent: paste it in one shot (no
# typing animation), submit, then start rolling so the first frame shows the
# prompt delivered to Jcode.
"$STAGE" paste "$PROMPT"
# The TUI drains a large paste over several frames; give it time to hold the
# complete text before submitting, then let the submit render so frame one
# already shows the prompt sent rather than sitting in the input box.
sleep 1.0
"$STAGE" key enter
sleep 1.5

"$STAGE" rec "$OUT"
T0=$(date +%s.%N)
mark() {
  local now
  now=$(date +%s.%N)
  awk -v now="$now" -v start="$T0" -v label="$1" \
    'BEGIN { printf "%.3f %s\n", now - start, label }' >> "$MARKS"
}
mark prompt
mark submitted

# The AIUC preview is focused as the first useful action. Its checked-in matrix
# writes a completion marker after all four measurements, so an early reveal
# cannot make the recorder stop before verification finishes. Other scenarios
# retain the state-preserving handoff boundary.
handed_off=0
preview_visible=0
for _ in $(seq 1 600); do
  windows=$("$STAGE_HWATU" list --json 2>/dev/null || printf '[]')
  if (( ! preview_visible )) && python3 - "$APP" "$windows" <<'PY' >/dev/null 2>&1
import json, sys
app, raw = sys.argv[1:]
windows = json.loads(raw)
raise SystemExit(0 if any(w.get('url') == app and w.get('mode') not in ('headless', 'background') for w in windows) else 1)
PY
  then
    preview_visible=1
    if [ "${HWATU_DEMO_SCENARIO:-stripe}" = aiuc ]; then
      # The daemon reports visible mode before the compositor has painted the
      # WebKit surface. Mark the edit point only after that first frame settles.
      sleep "${AIUC_PREVIEW_SETTLE_SECONDS:-10}"
      mark preview
    fi
  fi
  if [ "${HWATU_DEMO_SCENARIO:-stripe}" = aiuc ] && [ -f "$COMPLETE_FILE" ]; then
    handed_off=1
    break
  fi
  if [ "${HWATU_DEMO_SCENARIO:-stripe}" != aiuc ] && python3 - "$APP" "$windows" <<'PY' >/dev/null 2>&1
import json, sys
app, raw = sys.argv[1:]
windows = json.loads(raw)
raise SystemExit(0 if any(w.get('url') == app and w.get('mode') not in ('headless', 'background') for w in windows) else 1)
PY
  then
    handed_off=1
    break
  fi
  sleep 0.1
done
(( handed_off )) || fail "Jcode did not hand off the live app within 60 seconds"
(( preview_visible )) || fail "Jcode completed without revealing the live app"
mark handoff
# The browser becomes visible as soon as the focus tool completes. Keep rolling
# until Jcode has also streamed the score and returned to its idle prompt.
sleep 8.0

mark end
"$STAGE" stoprec
trap - EXIT HUP INT TERM
cleanup
printf 'raw film: %s\nmarkers:  %s\nJcode log: %s\n' "$OUT" "$MARKS" "$JCODE_LOG"
