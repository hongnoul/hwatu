#!/usr/bin/env bash
# Run the real hwatu CLI against the demo's isolated compositor and state.
# Jcode's server runs outside the compositor, so its tool subprocesses need all
# XDG paths, not only the Wayland socket, to avoid restoring a user's sessions.
set -euo pipefail

STAGE_DIR="${HWATU_DEMO_STAGE_DIR:-/tmp/hwatu-demo-stage}"
export XDG_RUNTIME_DIR="$STAGE_DIR/run"
export XDG_CONFIG_HOME="$STAGE_DIR/config"
export XDG_CACHE_HOME="$STAGE_DIR/cache"
export XDG_STATE_HOME="$STAGE_DIR/state"
export XDG_DATA_HOME="$STAGE_DIR/data"
export WAYLAND_DISPLAY="wayland-1"
unset DISPLAY

HWATU_BIN=${HWATU_DEMO_HWATU_BIN:-hwatu}
exec "$HWATU_BIN" "$@"
