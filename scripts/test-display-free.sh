#!/usr/bin/env bash
# Behavioral suite for display-free operation (roadmap G4): hwatud
# with no WAYLAND_DISPLAY/DISPLAY spawns a managed headless child
# compositor and serves the full automation surface. Runs against
# ISOLATED sockets/state dirs so the user's daemon is untouched.
#
# Asserts, per the roadmap test plan:
#   1. The daemon starts under `env -u WAYLAND_DISPLAY -u DISPLAY`
#      and reports display-free mode.
#   2. check / render / eval / shot all work display-free.
#   3. Shot pixels match a compositor-hosted run of the same fixture
#      (diff score >= 99%).
#   4. `hwatu focus <id>` returns the structured "no display" error,
#      not a crash (daemon still answers afterwards).
#   5. No orphan compositor survives daemon exit (clean quit AND
#      SIGKILL, which exercises the PDEATHSIG supervisor).
#
# Usage: scripts/test-display-free.sh
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$root/target/release"

if [[ ! -x "$bin/hwatu" || ! -x "$bin/hwatud" ]]; then
    echo "test-display-free: building release binaries..." >&2
    cargo build --release --manifest-path "$root/Cargo.toml" >&2
fi

# Keep the workdir path SHORT: it becomes XDG_RUNTIME_DIR, and Unix
# socket paths (compositor + daemon) cap at ~108 bytes.
work="$(mktemp -d /tmp/hwatu-df.XXXXXX)"

# GPU-less boxes (CI runners): WebKit's DMA-BUF renderer SIGTRAPs and
# GLES-on-llvmpipe has aborted cage without a DRM render node. The
# daemon applies this fallback itself in display-free mode; the
# REFERENCE half of this suite runs in session mode (script-spawned
# compositor + WAYLAND_DISPLAY), which by design changes nothing, so
# the script must set the same env there. Exported globally so both
# halves render identically for the pixel-parity assertion.
gpu=no
for node in /dev/dri/renderD*; do
    [[ -r "$node" && -w "$node" ]] && gpu=yes && break
done
if [[ "$gpu" == no ]]; then
    echo "test-display-free: no DRM render node; using software rendering" >&2
    export WEBKIT_DISABLE_DMABUF_RENDERER=1 LIBGL_ALWAYS_SOFTWARE=1 WLR_RENDERER=pixman
fi
comp_pid=""
cleanup() {
    [[ -n "$comp_pid" ]] && kill "$comp_pid" 2>/dev/null || true
    # Any daemon still running on either isolated socket.
    XDG_RUNTIME_DIR="$work/hosted" "$bin/hwatu" quit >/dev/null 2>&1 || true
    XDG_RUNTIME_DIR="$work/free" "$bin/hwatu" quit >/dev/null 2>&1 || true
    sleep 0.5
    rm -rf "$work"
}
trap cleanup EXIT

pass=0
fail=0
ok()  { pass=$((pass + 1)); echo "ok   - $1"; }
bad() { fail=$((fail + 1)); echo "FAIL - $1"; }
check() { # check <description> <condition...>
    local desc="$1"; shift
    if "$@"; then ok "$desc"; else bad "$desc"; fi
}

# The fixture both runs shot: deterministic, no network, no fonts
# beyond a solid block (color rectangles diff cleanly across runs).
fixture='<!DOCTYPE html><meta charset="utf-8"><title>df</title>
<body style="margin:0"><div style="width:100%;height:50vh;background:#1a6b3c"></div>
<div style="width:60%;height:50vh;background:#b03060"></div></body>'

mkdir -p "$work/hosted" "$work/free" "$work/state-hosted" "$work/state-free"

# ---- reference run: compositor-hosted ------------------------------
# Start our own headless compositor and hand its socket to the daemon
# via WAYLAND_DISPLAY, i.e. the "display present" path. This also
# works on CI, where no session compositor exists.
comp_log="$work/comp.log"
if command -v cage >/dev/null; then
    env -u WAYLAND_DISPLAY -u DISPLAY XDG_RUNTIME_DIR="$work/hosted" \
        WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER_ALLOW_SOFTWARE=1 \
        cage -- sleep infinity >"$comp_log" 2>&1 &
    comp_pid=$!
elif command -v labwc >/dev/null; then
    env -u WAYLAND_DISPLAY -u DISPLAY XDG_RUNTIME_DIR="$work/hosted" \
        WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER_ALLOW_SOFTWARE=1 \
        labwc >"$comp_log" 2>&1 &
    comp_pid=$!
elif command -v sway >/dev/null; then
    printf '# empty\n' > "$work/sway.conf"
    env -u WAYLAND_DISPLAY -u DISPLAY XDG_RUNTIME_DIR="$work/hosted" \
        WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER_ALLOW_SOFTWARE=1 \
        sway --config "$work/sway.conf" >"$comp_log" 2>&1 &
    comp_pid=$!
else
    echo "test-display-free: no headless compositor (cage/labwc/sway); cannot run" >&2
    exit 1
fi
sock=""
for _ in $(seq 200); do
    sock="$(find "$work/hosted" -maxdepth 1 -name 'wayland-*' ! -name '*.lock' | head -1)"
    [[ -n "$sock" && -S "$sock" ]] && break
    sleep 0.05
done
check "reference compositor came up" test -n "$sock"

env -u DISPLAY XDG_RUNTIME_DIR="$work/hosted" XDG_STATE_HOME="$work/state-hosted" \
    WAYLAND_DISPLAY="$sock" "$bin/hwatud" >"$work/hosted-daemon.log" 2>&1 &
for _ in $(seq 200); do [[ -S "$work/hosted/hwatu.sock" ]] && break; sleep 0.05; done
check "hosted daemon is NOT display-free (log)" \
    bash -c "! grep -q 'display-free mode' '$work/hosted-daemon.log'"

ref="$work/ref.png"
XDG_RUNTIME_DIR="$work/hosted" "$bin/hwatu" render --stdin --shot="$ref" <<<"$fixture" >/dev/null
check "hosted run produced a reference shot" test -s "$ref"
XDG_RUNTIME_DIR="$work/hosted" "$bin/hwatu" quit >/dev/null
kill "$comp_pid" 2>/dev/null || true
comp_pid=""
sleep 0.5

# ---- display-free run ----------------------------------------------
env -u WAYLAND_DISPLAY -u DISPLAY XDG_RUNTIME_DIR="$work/free" \
    XDG_STATE_HOME="$work/state-free" \
    "$bin/hwatud" >"$work/free-daemon.log" 2>&1 &
free_pid=$!
for _ in $(seq 200); do [[ -S "$work/free/hwatu.sock" ]] && break; sleep 0.05; done
check "display-free daemon came up on an isolated socket" \
    test -S "$work/free/hwatu.sock"
check "daemon reports display-free mode" \
    grep -q "display-free mode" "$work/free-daemon.log"

hw() { env -u WAYLAND_DISPLAY -u DISPLAY XDG_RUNTIME_DIR="$work/free" "$bin/hwatu" "$@"; }

# 2. check / render / eval / shot
out="$(hw render --stdin --eval "2 + 40" <<<"$fixture")"
check "render + eval work display-free" grep -q '"eval":42' <<<"$out"

port=8642
python3 -m http.server "$port" --bind 127.0.0.1 --directory "$work" >"$work/http.log" 2>&1 &
http_pid=$!
printf '%s' "$fixture" > "$work/df.html"
for _ in $(seq 50); do
    curl -sf -o "$work/curl.out" "http://127.0.0.1:$port/df.html" && break
    sleep 0.1
done
out="$(hw check "http://127.0.0.1:$port/df.html" --until dom)"
kill "$http_pid" 2>/dev/null || true
check "check works display-free (title seen)" grep -q '"title":"df"' <<<"$out"

shot="$work/free.png"
out="$(hw render --stdin --shot="$shot" <<<"$fixture")"
check "shot works display-free" test -s "$shot"

# 3. pixel parity with the compositor-hosted reference
out="$(hw render --stdin --baseline "$ref" <<<"$fixture")"
match="$(grep -o '"match_percent":[0-9.]*' <<<"$out" | cut -d: -f2)"
check "display-free pixels match hosted run (>= 99%, got ${match:-none})" \
    awk "BEGIN { exit !(${match:-0} >= 99) }"

# 4. focus returns the structured no-display error, daemon survives
id="$(hw render --stdin --keep <<<"$fixture" | grep -o '"id":[0-9]*' | cut -d: -f2)"
if err="$(hw focus "${id:-1}" 2>&1)"; then
    bad "focus fails display-free"
else
    check "focus error names the missing display" grep -q "no display" <<<"$err"
fi
out="$(hw ping)"
check "daemon still answers after the focus error" grep -q '"build"' <<<"$out"

# 5a. clean quit leaves no compositor
hw quit >/dev/null
sleep 1
if pgrep -f "hwatud-comp-$free_pid" >/dev/null 2>&1; then
    bad "no orphan compositor after clean quit"
else
    ok "no orphan compositor after clean quit"
fi

# 5b. SIGKILL'd daemon leaves no compositor (PDEATHSIG supervisor)
env -u WAYLAND_DISPLAY -u DISPLAY XDG_RUNTIME_DIR="$work/free" \
    XDG_STATE_HOME="$work/state-free" \
    "$bin/hwatud" >"$work/free-daemon2.log" 2>&1 &
free_pid=$!
disown "$free_pid" # suppress bash's "Killed" job notice
for _ in $(seq 200); do [[ -S "$work/free/hwatu.sock" ]] && break; sleep 0.05; done
kill -9 "$free_pid"
sleep 1.5
if pgrep -f "hwatud-comp-$free_pid" >/dev/null 2>&1; then
    bad "no orphan compositor after SIGKILL"
else
    ok "no orphan compositor after SIGKILL"
fi

echo
echo "test-display-free: $pass passed, $fail failed"
[[ "$fail" -eq 0 ]]
