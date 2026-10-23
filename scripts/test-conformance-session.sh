#!/usr/bin/env bash
# Linux only. Runs scripts/test-conformance.sh with the daemon on a Wayland
# session (headless sway standing in for the user's desktop) instead of in
# display-free mode. Fails if any window appears on that session: headless
# trusted input must use the daemon's private compositor.
# Needs sway and wtype. Usage: scripts/test-conformance-session.sh [release-bin-dir]
set -u
root="$(cd "$(dirname "$0")/.." && pwd)"
u=$(mktemp -d /tmp/hw-sess.XXXX)
chmod 700 "$u"
printf 'output * mode --custom 1600x900\n' > "$u/sway.conf"
XDG_RUNTIME_DIR="$u" WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 \
  WLR_RENDERER=pixman sway --config "$u/sway.conf" >"$u/sway.log" 2>&1 &
sw=$!
trap 'kill $sw 2>/dev/null; rm -rf "$u"' EXIT
for _ in $(seq 100); do
  sock=$(ls "$u" 2>/dev/null | grep '^wayland-[0-9]*$' | head -1)
  ipc=$(ls "$u"/sway-ipc.* 2>/dev/null | head -1)
  [[ -n "$sock" && -n "$ipc" ]] && break
  sleep 0.1
done
[[ -n "${sock:-}" ]] || { echo "sway did not start"; cat "$u/sway.log"; exit 1; }

# Record the most windows seen on the session while the suite runs,
# except while the suite's one intended `show` is in progress.
( max=0
  while kill -0 $sw 2>/dev/null; do
    [[ -e "$u/showing" ]] && { sleep 0.05; continue; }
    n=$(XDG_RUNTIME_DIR="$u" SWAYSOCK="$ipc" swaymsg -t get_tree -r 2>/dev/null | grep -c '"pid"')
    (( n > max )) && { max=$n; echo "$max" > "$u/max"; }
    sleep 0.05
  done ) &
watch=$!

HWATU_CONFORMANCE_SHOWING="$u/showing" HWATU_CONFORMANCE_SESSION=1 WAYLAND_DISPLAY="$u/$sock" "$root/scripts/test-conformance.sh" "$@"
rc=$?
kill $watch 2>/dev/null
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
