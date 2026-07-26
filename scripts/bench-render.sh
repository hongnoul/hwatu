#!/usr/bin/env bash
# Measure `hwatu render` (inline markup, no HTTP) against `hwatu check`
# of a loopback URL serving identical markup. Roadmap G1's claim is
# that render wins by skipping the HTTP roundtrip; this script is the
# measured-not-estimated evidence for benchmarks.md.
#
# Usage: scripts/bench-render.sh [iterations]   (default 20)
set -euo pipefail

iters="${1:-20}"
root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$root/target/release"

if [[ ! -x "$bin/hwatu" || ! -x "$bin/hwatud" ]]; then
    echo "bench-render: building release binaries..." >&2
    cargo build --release --manifest-path "$root/Cargo.toml" >&2
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/hwatu-render-bench.XXXXXX")"
export XDG_RUNTIME_DIR="$work/run"
export XDG_STATE_HOME="$work/state"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_STATE_HOME"

server_pid=""
cleanup() {
    "$bin/hwatu" quit >/dev/null 2>&1 || true
    [[ -n "$server_pid" ]] && kill "$server_pid" 2>/dev/null || true
    rm -rf "$work"
}
trap cleanup EXIT

# The same fixture markup the vs-playwright bench uses in spirit: a
# small static page, identical bytes for both paths.
page="$work/page.html"
cat > "$page" <<'HTML'
<!DOCTYPE html><html><head><meta charset="utf-8"><title>bench</title>
<style>body{font:16px sans-serif;margin:40px}h1{color:#345}</style>
</head><body><h1>hwatu render bench</h1>
<p>identical markup for render and check.</p>
<ul><li>one</li><li>two</li><li>three</li></ul>
</body></html>
HTML

port=8642
# No subshell: $! must be the python pid itself, or cleanup leaks it.
python3 -m http.server "$port" --bind 127.0.0.1 --directory "$work" >/dev/null 2>&1 &
server_pid=$!
for _ in $(seq 50); do
    curl -sf -o /dev/null "http://127.0.0.1:$port/page.html" && break
    sleep 0.1
done

"$bin/hwatu" ping >/dev/null # spawn isolated daemon
# Warm both paths once (first pass pays window construction).
"$bin/hwatu" render "$page" --shot >/dev/null
"$bin/hwatu" check "http://127.0.0.1:$port/page.html" --shot >/dev/null

median() { sort -n | awk '{ a[NR] = $1 } END { print a[int((NR + 1) / 2)] }'; }

render_ms=()
for _ in $(seq "$iters"); do
    t0=$(date +%s%N)
    "$bin/hwatu" render "$page" --shot >/dev/null
    render_ms+=($(( ($(date +%s%N) - t0) / 1000000 )))
done

check_ms=()
for _ in $(seq "$iters"); do
    t0=$(date +%s%N)
    "$bin/hwatu" check "http://127.0.0.1:$port/page.html" --shot >/dev/null
    check_ms+=($(( ($(date +%s%N) - t0) / 1000000 )))
done

render_med="$(printf '%s\n' "${render_ms[@]}" | median)"
check_med="$(printf '%s\n' "${check_ms[@]}" | median)"

echo "render->shot over $iters runs (ms): ${render_ms[*]}"
echo "check->shot  over $iters runs (ms): ${check_ms[*]}"
echo "median render->shot: ${render_med} ms"
echo "median check->shot:  ${check_med} ms (same markup over loopback HTTP)"
