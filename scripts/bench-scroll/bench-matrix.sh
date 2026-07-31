#!/usr/bin/env bash
# Run the scroll benchmark across a matrix of env configs.
# Each config gets a fresh isolated hwatud (private XDG_RUNTIME_DIR socket).
# Usage: bench-matrix.sh [results.jsonl]
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
OUT="${1:-$HERE/results.jsonl}"
SCRATCH="${BENCH_SCRATCH:-${JCODE_SCRATCH_DIR:-/tmp}/hwatu-scroll-bench}"
HWATUD="${HWATUD_BIN:-$HOME/.local/bin/hwatud}"
mkdir -p "$SCRATCH"
: > "$OUT"

bench_config() {
  local label="$1"; shift
  local dir="$SCRATCH/$label"
  mkdir -p "$dir"
  rm -f "$dir/hwatu.sock"
  echo "=== $label: $* ===" >&2
  env XDG_RUNTIME_DIR="$dir" WAYLAND_DISPLAY="$WAYLAND_DISPLAY" \
      HWATU_AGENT_MODE=normal HWATU_DISCARD_SECS=9999 "$@" \
      "$HWATUD" > "$dir/daemon.log" 2>&1 &
  local pid=$!
  for _ in $(seq 1 50); do
    [ -S "$dir/hwatu.sock" ] && break
    sleep 0.2
  done
  if [ ! -S "$dir/hwatu.sock" ]; then
    echo "{\"label\": \"$label\", \"error\": \"daemon failed to start\"}" >> "$OUT"
    kill "$pid" 2>/dev/null
    return
  fi
  sleep 1
  if ! "$HERE/bench-one.sh" "$dir" "$label" >> "$OUT" 2>"$dir/bench.err"; then
    echo "{\"label\": \"$label\", \"error\": \"bench failed: $(tail -1 "$dir/bench.err" | tr '"' "'")\"}" >> "$OUT"
  fi
  XDG_RUNTIME_DIR="$dir" hwatu quit >/dev/null 2>&1
  sleep 0.5
  kill "$pid" 2>/dev/null
  wait "$pid" 2>/dev/null
}

bench_config baseline HWATU_BENCH_NOOP=1
bench_config throttle-144 WEBKIT_DISPLAY_REFRESH_THROTTLE_FPS=144
bench_config damage-on WEBKIT_DISPLAY_REFRESH_THROTTLE_FPS=144 HWATU_WEBKIT_FEATURES=PropagateDamagingInformation:on
bench_config damage-show WEBKIT_DISPLAY_REFRESH_THROTTLE_FPS=144 WEBKIT_SHOW_DAMAGE=1 HWATU_WEBKIT_FEATURES=PropagateDamagingInformation:on
bench_config throttle-144-asyncscroll WEBKIT_DISPLAY_REFRESH_THROTTLE_FPS=144 HWATU_WEBKIT_FEATURES=AsyncFrameScrolling:on,ThreadedScrolling:on
bench_config kitchen-sink WEBKIT_DISPLAY_REFRESH_THROTTLE_FPS=144 HWATU_WEBKIT_FEATURES=AsyncFrameScrolling:on,ThreadedScrolling:on,PropagateDamagingInformation:on WEBKIT_SKIA_GPU_PAINTING_THREADS=4

echo "--- results ---"
cat "$OUT"
