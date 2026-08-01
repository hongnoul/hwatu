#!/usr/bin/env bash
set -euo pipefail

# Cross-origin iframe fixture for issue #23.
# Proves the old JS path is untrusted and cannot pierce a cross-origin
# iframe, and requires the trusted backend to produce event.isTrusted:true
# in both the top document and the child frame.

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
HWATU=${HWATU:-"$ROOT/target/debug/hwatu"}
PARENT_PORT=${PARENT_PORT:-8781}
CHILD_PORT=${CHILD_PORT:-8782}
BASE="http://127.0.0.1:${PARENT_PORT}/parent.html"
TMPDIR=${JCODE_SCRATCH_DIR:-${TMPDIR:-/tmp}}
RUN_DIR=$(mktemp -d "$TMPDIR/hwatu-trusted-input.XXXXXX")
export HWATU_SOCKET="$RUN_DIR/hwatu.sock"

if [[ ! -x "$HWATU" || ! -x "$ROOT/target/debug/hwatud" ]]; then
  cargo build --manifest-path "$ROOT/Cargo.toml" >/dev/null
fi

pids=()
cleanup() {
  for pid in "${pids[@]:-}"; do
    kill "$pid" 2>/dev/null || true
  done
  rm -rf "$RUN_DIR"
}
trap cleanup EXIT

python3 -m http.server "$PARENT_PORT" --bind 127.0.0.1 --directory "$ROOT/examples/trusted-input" >"$RUN_DIR/parent.log" 2>&1 &
pids+=("$!")
python3 -m http.server "$CHILD_PORT" --bind 127.0.0.1 --directory "$ROOT/examples/trusted-input" >"$RUN_DIR/child.log" 2>&1 &
pids+=("$!")
"$ROOT/target/debug/hwatud" >"$RUN_DIR/hwatud.log" 2>&1 &
pids+=("$!")
sleep 0.4

HWATU_AGENT_MODE=background "$HWATU" "$BASE" >/dev/null
"$HWATU" wait-load --until dom --timeout-ms 5000 >/dev/null

"$HWATU" click '#top-button' >/dev/null
top_trusted=$("$HWATU" eval 'return document.body.dataset.topTrusted' | tr -d '"')
if [[ "$top_trusted" != "false" ]]; then
  echo "FAIL: JS click should produce isTrusted:false on top button, got ${top_trusted}" >&2
  exit 1
fi

echo "ok: old JS click path isTrusted:false on same-origin top button"

if "$HWATU" click '#child-button' >"$RUN_DIR/child-click.out" 2>"$RUN_DIR/child-click.err"; then
  echo "FAIL: old JS click unexpectedly reached #child-button inside cross-origin iframe" >&2
  cat "$RUN_DIR/child-click.out" >&2
  exit 1
fi
if ! grep -q 'no match' "$RUN_DIR/child-click.err"; then
  echo "FAIL: expected old JS child click to fail with no match" >&2
  cat "$RUN_DIR/child-click.err" >&2
  exit 1
fi

echo "ok: old JS selector path cannot reach cross-origin iframe contents"

if "$HWATU" click --trusted '#top-button' >"$RUN_DIR/trusted-click.out" 2>"$RUN_DIR/trusted-click.err"; then
  top_trusted=$("$HWATU" eval 'return document.body.dataset.topTrusted' | tr -d '"')
  if [[ "$top_trusted" != "true" ]]; then
    echo "FAIL: trusted click completed but page saw isTrusted:${top_trusted}" >&2
    echo "      If a daemon was already running, restart it so it understands the trusted flag." >&2
    exit 1
  fi
  echo "ok: trusted click produced isTrusted:true"
else
  cat "$RUN_DIR/trusted-click.err" >&2
  exit 1
fi

"$HWATU" click --trusted '#child-button' >"$RUN_DIR/trusted-child-click.out" 2>"$RUN_DIR/trusted-child-click.err"
child_button=$("$HWATU" eval 'return JSON.parse(document.body.dataset.childButton || "null")')
if ! jq -e '.trusted == true' <<<"$child_button" >/dev/null; then
  echo "FAIL: trusted child click did not produce isTrusted:true in iframe" >&2
  echo "childButton=${child_button}" >&2
  cat "$RUN_DIR/trusted-child-click.err" >&2
  exit 1
fi

echo "ok: trusted click reached cross-origin iframe with isTrusted:true"

"$HWATU" type --trusted '#child-input' hwatu >"$RUN_DIR/trusted-child-type.out" 2>"$RUN_DIR/trusted-child-type.err"
child_input=$("$HWATU" eval 'return JSON.parse(document.body.dataset.childInput || "null")')
if ! jq -e '.trusted == true and .value == "hwatu"' <<<"$child_input" >/dev/null; then
  echo "FAIL: trusted child type did not produce isTrusted:true and value=hwatu in iframe" >&2
  echo "childInput=${child_input}" >&2
  cat "$RUN_DIR/trusted-child-type.err" >&2
  exit 1
fi

echo "ok: trusted type reached cross-origin iframe with isTrusted:true"
