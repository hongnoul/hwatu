#!/usr/bin/env bash
# gate-runner.sh REF_DIR CLONE_DIR OUT_JSON
#
# Phase 0 deterministic gate runner for the perfect-clone plan
# (.astrophile/drafts/perfect-clone-plan.md). Spins ISOLATED hwatud
# daemons (private XDG_RUNTIME_DIR => private socket; never touches
# the user's daemon), serves REF_DIR and CLONE_DIR over http, loads
# both headless, pins virtual time, and walks:
#
#   static:   widths {360,768,1024,1280,1528,1920} x DPR {1,2}
#             x scroll {0,25,50,75,100}%   at virtual t=0, tolerance 0
#   temporal: lockstep `clock step` to t in {250,1000,5000,30000,103960}
#   motion:   declared inventory + `motion --observe --ms 2500` on both
#
# Emits one scorecard JSON (arg 3). DPR is a per-process GTK property
# (GDK_SCALE), so the matrix runs one isolated daemon per DPR.
#
# Known hwatu gap worked around here: `hwatu resize WxH` measures the
# page's devicePixelRatio and allocates W*dpr logical px, but GTK
# logical px are already CSS px (the surface scale maps logical ->
# device). Under GDK_SCALE=2 a `resize 360x800` therefore lands the
# page at 720 CSS px. We request W/dpr and verify innerWidth from the
# page after every resize.
set -euo pipefail

REF_DIR=${1:?usage: gate-runner.sh REF_DIR CLONE_DIR OUT_JSON}
CLONE_DIR=${2:?usage: gate-runner.sh REF_DIR CLONE_DIR OUT_JSON}
OUT_JSON=${3:?usage: gate-runner.sh REF_DIR CLONE_DIR OUT_JSON}
REF_DIR=$(realpath "$REF_DIR"); CLONE_DIR=$(realpath "$CLONE_DIR")

require_fixture() {
  local kind=$1 dir=$2
  [ -d "$dir" ] || { echo "$kind fixture directory does not exist: $dir" >&2; exit 1; }
  [ -r "$dir/index.html" ] || { echo "$kind fixture has no readable index.html: $dir" >&2; exit 1; }
}
require_fixture reference "$REF_DIR"
require_fixture clone "$CLONE_DIR"

HERE=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$HERE/../.." && pwd)
HWATU_BIN=${HWATU_BIN:-$REPO_ROOT/target/release/hwatu}
[ -x "$HWATU_BIN" ] || { echo "hwatu binary not found at $HWATU_BIN (cargo build --release)" >&2; exit 1; }
export PATH=$(dirname "$HWATU_BIN"):$PATH

WIDTHS=(360 768 1024 1280 1528 1920)
DPRS=(1 2)
SCROLLS=(0 25 50 75 100)
TIMES=(250 1000 5000 30000 103960)  # last ~= one marquee wrap
HEIGHT=800
TOL=0
STATIC_ONLY=${STATIC_ONLY:-0}
# The reference chooses one of six visible stats themes with
# `new Date().getHours()`. Pin both epoch and timezone so a gate started
# on another machine or at another real hour renders the same pre-dawn
# specimen. 2026-07-24T06:00:00Z deliberately avoids every
# 5/8/11/16/20/23 variant boundary.
CLOCK_EPOCH_MS=${CLOCK_EPOCH_MS:-1784872800000}

WORK=$(mktemp -d "${TMPDIR:-/tmp}/gate-runner.XXXXXX")
CELLS="$WORK/cells.jsonl"; : >"$CELLS"
TEMPORAL="$WORK/temporal.jsonl"; : >"$TEMPORAL"
MOTION_JSON="$WORK/motion.json"; echo '{}' >"$MOTION_JSON"
PIDS=()
RTDIRS=()

cleanup() {
  for rt in "${RTDIRS[@]:-}"; do
    [ -n "$rt" ] && XDG_RUNTIME_DIR="$rt" "$HWATU_BIN" quit >/dev/null 2>&1 || true
  done
  sleep 0.3
  for p in "${PIDS[@]:-}"; do kill "$p" >/dev/null 2>&1 || true; done
  for rt in "${RTDIRS[@]:-}"; do [ -n "$rt" ] && rm -rf "$rt"; done
  rm -rf "$WORK"
}
trap cleanup EXIT

# ---- static file servers -------------------------------------------
pick_port() { python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'; }
REF_PORT=$(pick_port); CLONE_PORT=$(pick_port)
python3 -m http.server "$REF_PORT" --directory "$REF_DIR" --bind 127.0.0.1 >/dev/null 2>&1 &
PIDS+=($!)
python3 -m http.server "$CLONE_PORT" --directory "$CLONE_DIR" --bind 127.0.0.1 >/dev/null 2>&1 &
PIDS+=($!)
for i in $(seq 50); do
  curl -sf -o /dev/null "http://127.0.0.1:$REF_PORT/" && curl -sf -o /dev/null "http://127.0.0.1:$CLONE_PORT/" && break
  sleep 0.1
done
curl -sf -o /dev/null "http://127.0.0.1:$REF_PORT/" \
  || { echo "reference fixture server did not become ready: $REF_DIR" >&2; exit 1; }
curl -sf -o /dev/null "http://127.0.0.1:$CLONE_PORT/" \
  || { echo "clone fixture server did not become ready: $CLONE_DIR" >&2; exit 1; }

REF_URL="http://127.0.0.1:$REF_PORT/"
CLONE_URL="http://127.0.0.1:$CLONE_PORT/"

# hw <rtdir> <args...> : talk to the isolated daemon behind rtdir
hw() { local rt=$1; shift; XDG_RUNTIME_DIR="$rt" "$HWATU_BIN" "$@"; }

# ---- private display server ----------------------------------------
# DPR must be exact (1 and 2). The session compositor imposes its own
# output scale (e.g. niri at 1.25 -> WebKit reports dpr 2 regardless
# of GDK_SCALE on Wayland), so the runner brings its own Xvfb: under
# GDK_BACKEND=x11, GDK_SCALE maps 1:1 onto devicePixelRatio.
XVFB_DISPLAY=":$(( (RANDOM % 500) + 400 ))"
Xvfb "$XVFB_DISPLAY" -screen 0 4000x4000x24 >/dev/null 2>&1 &
PIDS+=($!)
for i in $(seq 50); do [ -e "/tmp/.X11-unix/X${XVFB_DISPLAY#:}" ] && break; sleep 0.1; done

# start_daemon <gdk_scale> -> echoes rtdir
start_daemon() {
  local scale=$1 rt
  rt=$(mktemp -d "${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/hwatu-gate.XXXXXX")
  RTDIRS+=("$rt")
  XDG_RUNTIME_DIR="$rt" GDK_BACKEND=x11 DISPLAY="$XVFB_DISPLAY" \
    WAYLAND_DISPLAY= GDK_SCALE="$scale" TZ=UTC \
    HWATU_CLOCK_EPOCH_MS="$CLOCK_EPOCH_MS" \
    "$HWATU_BIN" ping >/dev/null
  echo "$rt"
}

# resize both windows to <w> CSS px, verifying from the page.
# Works around the resize/dpr double-application (see header).
resize_both() {
  local rt=$1 w=$2 dpr=$3 req=$(( w / dpr )) reqh=$(( HEIGHT / dpr )) wid got
  for wid in "$REF_ID" "$CLONE_ID"; do
    hw "$rt" resize --id "$wid" "${req}x${reqh}" >/dev/null
    got=$(hw "$rt" eval --id "$wid" 'window.innerWidth')
    if [ "$got" != "$w" ]; then
      # Fall back: maybe resize is dpr-correct on this build.
      hw "$rt" resize --id "$wid" "${w}x${HEIGHT}" >/dev/null
      got=$(hw "$rt" eval --id "$wid" 'window.innerWidth')
      [ "$got" = "$w" ] || { echo "resize failed: want ${w} CSS px, page reports ${got} (dpr $dpr)" >&2; exit 1; }
    fi
  done
}

diff_pct() { # diff_pct <rt> -> echoes full diff json
  hw "$1" diff --id "$CLONE_ID" --other "$REF_ID" --tolerance "$TOL"
}

# ---- per-DPR static pass -------------------------------------------
for dpr in "${DPRS[@]}"; do
  echo "== daemon dpr=$dpr" >&2
  RT=$(start_daemon "$dpr")
  # Seed blank windows before loading either fixture. The seed persists
  # across navigation and keeps Math.random call order deterministic.
  seed_url='data:text/html,%3Ctitle%3Ehwatu-seed%3C%2Ftitle%3E'
  REF_ID=$(hw "$RT" --headless --json "$seed_url" | jq .id)
  CLONE_ID=$(hw "$RT" --headless --json "$seed_url" | jq .id)
  hw "$RT" wait-load --id "$REF_ID" --timeout-ms 60000 >/dev/null
  hw "$RT" wait-load --id "$CLONE_ID" --timeout-ms 60000 >/dev/null
  hw "$RT" clock --id "$REF_ID" seed 1 >/dev/null
  hw "$RT" clock --id "$CLONE_ID" seed 1 >/dev/null
  for w in "${WIDTHS[@]}"; do
    resize_both "$RT" "$w" "$dpr"
    # Each viewport is an independent t=0 specimen. A paused resize can
    # strand responsive hydration in the preceding viewport's rAF/timer
    # state, so navigate only after sizing the windows, then finish every
    # native-clock load/font wait before taking control of the new realm.
    hw "$RT" goto --id "$REF_ID" "$REF_URL" >/dev/null
    hw "$RT" goto --id "$CLONE_ID" "$CLONE_URL" >/dev/null
    hw "$RT" wait-load --id "$REF_ID" --timeout-ms 60000 >/dev/null
    hw "$RT" wait-load --id "$CLONE_ID" --timeout-ms 60000 >/dev/null
    for wid in "$REF_ID" "$CLONE_ID"; do
      hw "$RT" eval --id "$wid" 'await document.fonts.ready; return document.fonts.status' >/dev/null
      got_dpr=$(hw "$RT" eval --id "$wid" 'window.devicePixelRatio')
      [ "$got_dpr" = "$dpr" ] || { echo "expected dpr $dpr, page reports $got_dpr" >&2; exit 1; }
      hw "$RT" clock --id "$wid" set 0 >/dev/null
      st=$(hw "$RT" clock --id "$wid" status)
      [ "$(jq '.paused and .virtual_ms == 0' <<<"$st")" = true ] \
        || { echo "clock not paused at t=0 after viewport load: $st" >&2; exit 1; }
    done
    # Same absolute scroll y on both, derived from the ref's range.
    max_y=$(hw "$RT" eval --id "$REF_ID" 'Math.max(0, Math.round(document.documentElement.scrollHeight - window.innerHeight))')
    for pct in "${SCROLLS[@]}"; do
      y=$(( max_y * pct / 100 ))
      for wid in "$REF_ID" "$CLONE_ID"; do
        hw "$RT" eval --id "$wid" "window.scrollTo(0, $y); return window.scrollY" >/dev/null
      done
      d=$(diff_pct "$RT")
      jq -c --argjson w "$w" --argjson dpr "$dpr" --argjson scroll "$pct" \
        '{w:$w, dpr:$dpr, scroll:$scroll, pct:.match_percent, mismatched:.mismatched_pixels, total:.total_pixels}' \
        <<<"$d" >>"$CELLS"
      echo "  w=$w dpr=$dpr scroll=$pct% -> $(jq .match_percent <<<"$d")%" >&2
    done
  done

  # ---- temporal + motion, run once, on the dpr=1 daemon -------------
  if [ "$dpr" = 1 ] && [ "$STATIC_ONLY" != 1 ]; then
    resize_both "$RT" 1280 1
    for wid in "$REF_ID" "$CLONE_ID"; do
      hw "$RT" eval --id "$wid" 'window.scrollTo(0,0); return 0' >/dev/null
    done
    d=$(diff_pct "$RT")
    jq -c '{t:0, pct:.match_percent, mismatched:.mismatched_pixels}' <<<"$d" >>"$TEMPORAL"
    prev=0
    for t in "${TIMES[@]}"; do
      step=$(( t - prev )); prev=$t
      hw "$RT" clock --id "$REF_ID" step "$step" --timeout-ms 300000 >/dev/null
      hw "$RT" clock --id "$CLONE_ID" step "$step" --timeout-ms 300000 >/dev/null
      d=$(diff_pct "$RT")
      jq -c --argjson t "$t" '{t:$t, pct:.match_percent, mismatched:.mismatched_pixels}' <<<"$d" >>"$TEMPORAL"
      echo "  t=${t}ms -> $(jq .match_percent <<<"$d")%" >&2
    done
    # Motion pass last: --observe perturbs the timeline.
    ref_decl=$(hw "$RT" motion --id "$REF_ID")
    clone_decl=$(hw "$RT" motion --id "$CLONE_ID")
    ref_obs=$(hw "$RT" motion --id "$REF_ID" --observe --ms 2500)
    clone_obs=$(hw "$RT" motion --id "$CLONE_ID" --observe --ms 2500)
    jq -n --argjson rd "$ref_decl" --argjson cd "$clone_decl" \
          --argjson ro "$ref_obs" --argjson co "$clone_obs" \
      '{declared_ref:$rd, declared_clone:$cd,
        fits:{ref:($ro.observed // []), clone:($co.observed // []),
              ref_meta:($ro.observed_meta // null), clone_meta:($co.observed_meta // null)}}' \
      >"$MOTION_JSON"
  fi

  hw "$RT" quit >/dev/null 2>&1 || true
done

# ---- assemble scorecard --------------------------------------------
REF_HASH=$(cd "$REF_DIR" && find . -type f -print0 | sort -z | xargs -0 sha256sum | sha256sum | cut -d' ' -f1)
HWATU_VERSION=$("$HWATU_BIN" ping 2>/dev/null | jq -r '"\(.version)+\(.build)"' || echo unknown)

jq -n \
  --arg ref_hash "$REF_HASH" \
  --arg date "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg hv "$HWATU_VERSION" \
  --arg ref_dir "$REF_DIR" --arg clone_dir "$CLONE_DIR" \
  --slurpfile static <(jq -c . "$CELLS") \
  --slurpfile temporal <(jq -c . "$TEMPORAL") \
  --slurpfile motion "$MOTION_JSON" \
  '{meta:{ref_hash:$ref_hash, date:$date, hwatu_version:$hv,
          ref_dir:$ref_dir, clone_dir:$clone_dir,
          tolerance:0, height:'"$HEIGHT"'},
    static:$static, temporal:$temporal, motion:$motion[0]}' \
  >"$OUT_JSON"

echo "scorecard: $OUT_JSON" >&2
jq '{static_min:([.static[].pct]|min), temporal_min:([.temporal[].pct]|min), cells:(.static|length)}' "$OUT_JSON" >&2
