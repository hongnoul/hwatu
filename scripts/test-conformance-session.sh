#!/usr/bin/env bash
# Linux only. Runs scripts/test-conformance.sh with the daemon on a Wayland
# session (a headless compositor standing in for the user's desktop)
# instead of in display-free mode.
# HWATU_SESSION=sway (default): wlroots, like sway, niri, Hyprland. Fails if
#   any window appears on the session: headless trusted input must use the
#   daemon's private compositor.
# HWATU_SESSION=mutter: GNOME's compositor, which takes no virtual input.
#   Window appearance is not watched (no IPC to count windows).
# Needs sway (also the private compositor), wtype, and mutter for that mode.
# Usage: scripts/test-conformance-session.sh [release-bin-dir]
set -u
root="$(cd "$(dirname "$0")/.." && pwd)"
session="${HWATU_SESSION:-sway}"
u=$(mktemp -d /tmp/hw-sess.XXXX)
chmod 700 "$u"
ipc=
case "$session" in
  sway)
    printf 'output * mode --custom 1600x900\n' > "$u/sway.conf"
    XDG_RUNTIME_DIR="$u" WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 \
      WLR_RENDERER=pixman sway --config "$u/sway.conf" >"$u/comp.log" 2>&1 & ;;
  mutter)
    XDG_RUNTIME_DIR="$u" dbus-run-session -- mutter --headless --wayland --no-x11 \
      --virtual-monitor 1600x900 --wayland-display wayland-9 >"$u/comp.log" 2>&1 & ;;
  *) echo "unknown HWATU_SESSION=$session"; exit 2 ;;
esac
sw=$!
trap 'kill $sw 2>/dev/null; rm -rf "$u"' EXIT
for _ in $(seq 100); do
  sock=$(ls "$u" 2>/dev/null | grep '^wayland-[0-9]*$' | head -1)
  [[ "$session" == sway ]] && ipc=$(ls "$u"/sway-ipc.* 2>/dev/null | head -1)
  [[ -n "$sock" && ( "$session" != sway || -n "$ipc" ) ]] && break
  sleep 0.1
done
[[ -n "${sock:-}" ]] || { echo "$session did not start"; cat "$u/comp.log"; exit 1; }

# Record the most windows seen on the session while the suite runs,
# except while the suite's one intended `show` is in progress.
[[ "$session" == sway ]] && ( max=0
  while kill -0 $sw 2>/dev/null; do
    [[ -e "$u/showing" ]] && { sleep 0.05; continue; }
    n=$(XDG_RUNTIME_DIR="$u" SWAYSOCK="$ipc" swaymsg -t get_tree -r 2>/dev/null | grep -c '"pid"')
    (( n > max )) && { max=$n; echo "$max" > "$u/max"; }
    sleep 0.05
  done ) &
watch=$!
[[ "$session" == sway ]] || { kill $watch 2>/dev/null; }

HWATU_CONFORMANCE_SHOWING="$u/showing" HWATU_CONFORMANCE_SESSION=1 WAYLAND_DISPLAY="$u/$sock" "$root/scripts/test-conformance.sh" "$@"
rc=$?
kill $watch 2>/dev/null
[[ "$session" == sway ]] || { echo "($session: window appearance not watched)"; exit $rc; }
max=$(cat "$u/max" 2>/dev/null || echo 0)
if (( max > 0 )); then
  echo "FAIL - $max window(s) appeared on the session during the run"
  exit 1
fi
echo "ok   - no window appeared on the session"

# Control: the count above must be able to see a window. `show` maps one.
b="${1:-$root/target/release}"
(
  export XDG_RUNTIME_DIR="$u" WAYLAND_DISPLAY="$u/$sock" HWATU_SOCKET="$u/control.sock"
  export WEBKIT_DISABLE_DMABUF_RENDERER=1 LIBGL_ALWAYS_SOFTWARE=1
  unset DISPLAY
  id=$("$b/hwatu" 'data:text/html,control' | grep -o 'window [0-9]*' | cut -d' ' -f2)
  "$b/hwatu" show "$id" >/dev/null
  sleep 0.5
  n=$(SWAYSOCK="$ipc" swaymsg -t get_tree -r | grep -c '"pid"')
  "$b/hwatu" quit >/dev/null 2>&1
  if (( n > 0 )); then echo "ok   - control: show maps a window on the session"
  else echo "FAIL - control: show mapped nothing, so the count above proves nothing"; exit 1; fi
) || exit 1
exit $rc
