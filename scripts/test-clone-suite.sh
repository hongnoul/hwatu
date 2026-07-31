#!/usr/bin/env bash
# Fidelity ladder for `hwatu clone`: the canonical difficulty
# progression every clone phase must climb, in order.
#
#   1. webkit.org — static-ish marketing page: CSSOM inlining, SVG
#      assets, web fonts. The floor: if this regresses, everything
#      else is noise.
#   2. stripe.com — the original 100%-match demo target: WebGL
#      canvases, entrance reveals, transition pins, scroll-snap
#      carousels, cross-origin CSS.
#   3. scale.com — heavy JS-built DOM, aggressive lazy loading,
#      script-driven motion. The current frontier.
#
# Each rung runs `hwatu clone` on an ISOLATED daemon (the user's
# session is untouched), records the verify report, and asserts the
# average pixel match clears the rung's threshold. Thresholds are
# floors, not goals: raise them as later phases (resource archive,
# animation re-arm) land, never lower them to pass.
#
# Live-site caveat, stated honestly: these are third-party pages that
# change without notice. A failure here means "look", not necessarily
# "the code broke" — read the report and heatmap regions before
# blaming the pipeline. CI should treat this suite as advisory
# (network + third-party variance); the gate is local pre-merge runs.
#
# Usage: scripts/test-clone-suite.sh [outdir]
#   outdir defaults to a mktemp dir; pass one to keep the clones.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$root/target/release"

if [[ ! -x "$bin/hwatu" || ! -x "$bin/hwatud" ]]; then
    echo "test-clone-suite: building release binaries..." >&2
    cargo build --release --manifest-path "$root/Cargo.toml" >&2
fi

keep_out=""
if [[ $# -ge 1 ]]; then
    keep_out="$1"
    mkdir -p "$keep_out"
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/hwatu-clone-suite.XXXXXX")"
out="${keep_out:-$work/clones}"
mkdir -p "$out"
export XDG_RUNTIME_DIR="$work/run"
export XDG_STATE_HOME="$work/state"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_STATE_HOME"
# Deny media autoplay in the capture daemon. WebKitGTK+GStreamer
# (gst 1.28.5) deadlocks the web process main thread on pages with
# several lazy-initialized autoplay videos (scale.com); a still
# clone renders poster frames anyway, so playback buys nothing.
export HWATU_BLOCK_AUTOPLAY=1

cleanup() {
    "$bin/hwatu" quit >/dev/null 2>&1 || true
    rm -rf "$work"
}
trap cleanup EXIT

# The ladder: url  min_avg_match  viewport
# Thresholds reflect what Phase 0 (stills) demonstrably achieves
# today; see the table in the results log for measured values.
rungs=(
    "https://webkit.org   90.0  1440x900"
    "https://stripe.com   85.0  1920x1080"
    "https://scale.com    75.0  1920x1080"
)

fails=0
summary=()
for rung in "${rungs[@]}"; do
    read -r url min viewport <<<"$rung"
    name="$(sed -E 's#https?://##; s#[/.]#-#g' <<<"$url")"
    dir="$out/$name"
    echo "=== clone $url (floor ${min}%) ===" >&2
    if ! "$bin/hwatu" clone "$url" --out "$dir" --viewport "$viewport" \
        --timeout-ms 240000 >&2; then
        echo "FAIL $url: clone exited nonzero" >&2
        summary+=("FAIL  $url  (clone error)")
        fails=$((fails + 1))
        continue
    fi
    avg="$(python3 -c "
import json, sys
r = json.load(open('$dir/report.json'))
print(r['average_match_percent'])
")"
    ok="$(python3 -c "print('yes' if $avg >= $min else 'no')")"
    if [[ "$ok" == "yes" ]]; then
        summary+=("PASS  $url  avg=${avg}%  floor=${min}%")
    else
        summary+=("FAIL  $url  avg=${avg}%  floor=${min}%")
        fails=$((fails + 1))
    fi
done

echo
echo "== clone fidelity ladder =="
printf '%s\n' "${summary[@]}"
[[ -n "$keep_out" ]] && echo "clones kept in $keep_out"
exit "$fails"
