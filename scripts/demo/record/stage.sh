#!/usr/bin/env bash
# stage.sh — bring up (and tear down) the invisible demo stage.
#
# Creates a fully headless recording environment that never touches
# the user's desktop:
#   - nested sway on the wlroots headless backend (1920x1080)
#   - a dedicated XDG_RUNTIME_DIR so sockets never collide
#   - an isolated hwatud instance inside the stage
#   - a kitty terminal with remote control (the "director" types
#     into it programmatically)
#   - wf-recorder capturing HEADLESS-1 to mp4
#
# Usage:
#   scripts/demo/record/stage.sh up        # start compositor + terminal
#   scripts/demo/record/stage.sh rec FILE  # start recording to FILE
#   scripts/demo/record/stage.sh stoprec   # stop recording (flushes mp4)
#   scripts/demo/record/stage.sh shot FILE # single PNG frame (grim)
#   scripts/demo/record/stage.sh type CMD  # type + run CMD in the terminal
#   scripts/demo/record/stage.sh text TEXT # type without submitting
#   scripts/demo/record/stage.sh key KEY   # send a raw key (e.g. enter, ctrl+c)
#   scripts/demo/record/stage.sh env       # print env exports for manual use
#   scripts/demo/record/stage.sh down      # tear everything down
set -euo pipefail

STAGE_DIR="${HWATU_DEMO_STAGE_DIR:-/tmp/hwatu-demo-stage}"
RES="${HWATU_DEMO_RES:-1920x1080}"
CONFIG_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Typing rhythm (seconds between keystrokes) for human-feel typing.
TYPE_DELAY="${HWATU_DEMO_TYPE_DELAY:-0.03}"

stage_env() {
  export XDG_RUNTIME_DIR="$STAGE_DIR/run"
  export XDG_CONFIG_HOME="$STAGE_DIR/config"
  export XDG_CACHE_HOME="$STAGE_DIR/cache"
  export XDG_STATE_HOME="$STAGE_DIR/state"
  export XDG_DATA_HOME="$STAGE_DIR/data"
  export WAYLAND_DISPLAY="wayland-1"
  export HWATU_DEMO_KITTY_SOCK="$STAGE_DIR/kitty.sock"
  export SWAYSOCK="$(ls "$STAGE_DIR"/run/sway-ipc.*.sock 2>/dev/null | head -1 || true)"
  # Keep the staged hwatu daemon fully isolated from the user's.
  unset DISPLAY
}

up() {
  if [ -e "$STAGE_DIR/sway.pid" ] && kill -0 "$(cat "$STAGE_DIR/sway.pid")" 2>/dev/null; then
    echo "stage already up (sway pid $(cat "$STAGE_DIR/sway.pid"))"
    return 0
  fi
  rm -rf "$STAGE_DIR"
  mkdir -p -m 700 "$STAGE_DIR"/{run,config,cache,state,data}

  export HWATU_DEMO_KITTY_SOCK="$STAGE_DIR/kitty.sock"
  WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=pixman \
    XDG_RUNTIME_DIR="$STAGE_DIR/run" \
    XDG_CONFIG_HOME="$STAGE_DIR/config" XDG_CACHE_HOME="$STAGE_DIR/cache" \
    XDG_STATE_HOME="$STAGE_DIR/state" XDG_DATA_HOME="$STAGE_DIR/data" \
    sway -c "$CONFIG_DIR/sway.config" >"$STAGE_DIR/sway.log" 2>&1 &
  echo $! > "$STAGE_DIR/sway.pid"

  # Wait for the display, then for kitty's control socket.
  for _ in $(seq 1 50); do
    [ -e "$STAGE_DIR/run/wayland-1" ] && break; sleep 0.1
  done
  [ -e "$STAGE_DIR/run/wayland-1" ] || { echo "sway failed:"; tail -5 "$STAGE_DIR/sway.log"; exit 1; }
  for _ in $(seq 1 50); do
    [ -e "$HWATU_DEMO_KITTY_SOCK" ] && break; sleep 0.1
  done
  [ -e "$HWATU_DEMO_KITTY_SOCK" ] || { echo "kitty socket missing"; exit 1; }

  # Pin DPR to 1 for reproducible shots, isolate the daemon socket.
  stage_env
  echo "stage up: WAYLAND_DISPLAY=wayland-1 XDG_RUNTIME_DIR=$STAGE_DIR/run"
  echo "hwatu daemon will autostart inside the stage on first 'type' command."
}

rec() {
  local out="${1:?usage: stage.sh rec out.mp4}"
  stage_env
  wf-recorder -o HEADLESS-1 -r 60 -f "$out" \
    -c libx264 -p crf=18 -p preset=slow >"$STAGE_DIR/rec.log" 2>&1 &
  echo $! > "$STAGE_DIR/rec.pid"
  sleep 0.5
  kill -0 "$(cat "$STAGE_DIR/rec.pid")" || { echo "recorder died:"; tail -3 "$STAGE_DIR/rec.log"; exit 1; }
  echo "recording HEADLESS-1 -> $out"
}

stoprec() {
  [ -e "$STAGE_DIR/rec.pid" ] || { echo "not recording"; exit 1; }
  kill -INT "$(cat "$STAGE_DIR/rec.pid")" 2>/dev/null || true
  wait "$(cat "$STAGE_DIR/rec.pid")" 2>/dev/null || true
  rm -f "$STAGE_DIR/rec.pid"
  echo "recording stopped"
}

shot() {
  local out="${1:?usage: stage.sh shot out.png}"
  stage_env
  grim -o HEADLESS-1 "$out"
  echo "frame -> $out"
}

# Type text into the director terminal with a human rhythm. Sending a real Enter
# key separately matters for raw-mode applications such as the Jcode TUI;
# injecting a newline byte only submits reliably in a shell.
type_text() {
  local text="${1:?usage: stage.sh text 'text'}"
  stage_env
  local i ch
  for (( i=0; i<${#text}; i++ )); do
    ch="${text:$i:1}"
    kitty @ --to "unix:$HWATU_DEMO_KITTY_SOCK" send-text -- "$ch"
    sleep "$TYPE_DELAY"
  done
}

type_cmd() {
  type_text "${1:?usage: stage.sh type 'command'}"
  sleep 0.1
  press_key enter
}

press_key() {
  local key="${1:?usage: stage.sh key KEY}"
  stage_env
  kitty @ --to "unix:$HWATU_DEMO_KITTY_SOCK" send-key "$key"
}

down() {
  stage_env 2>/dev/null || true
  [ -e "$STAGE_DIR/rec.pid" ] && stoprec || true
  # Ask the staged hwatu daemon to quit before killing the compositor.
  hwatu quit >/dev/null 2>&1 || true
  [ -e "$STAGE_DIR/sway.pid" ] && kill "$(cat "$STAGE_DIR/sway.pid")" 2>/dev/null || true
  rm -rf "$STAGE_DIR"
  echo "stage down"
}

case "${1:-}" in
  up) up ;;
  rec) rec "${2:-}" ;;
  stoprec) stoprec ;;
  shot) shot "${2:-}" ;;
  type) type_cmd "${2:-}" ;;
  text) type_text "${2:-}" ;;
  key) press_key "${2:-}" ;;
  env) echo "export XDG_RUNTIME_DIR=$STAGE_DIR/run XDG_CONFIG_HOME=$STAGE_DIR/config XDG_CACHE_HOME=$STAGE_DIR/cache XDG_STATE_HOME=$STAGE_DIR/state XDG_DATA_HOME=$STAGE_DIR/data WAYLAND_DISPLAY=wayland-1 SWAYSOCK=\$(ls $STAGE_DIR/run/sway-ipc.*.sock)" ;;
  down) down ;;
  *) sed -n '2,20p' "$0"; exit 1 ;;
esac
