#!/usr/bin/env bash
# One-off measurement for the 5c window-strategy note: sweep on one
# resized pooled window vs N separate fresh checks. Not part of CI.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$root/target/release"
work="$(mktemp -d "${TMPDIR:-/tmp}/hwatu-vp-bench.XXXXXX")"
export XDG_RUNTIME_DIR="$work/run" XDG_STATE_HOME="$work/state"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_STATE_HOME"
cat > "$work/resp.html" <<'HTML'
<!DOCTYPE html><meta charset="utf-8"><title>bench</title>
<style>@media (min-width:768px){body{background:#dfe8ff}}</style><div>x</div>
HTML
python3 -m http.server 8643 --bind 127.0.0.1 --directory "$work" >"$work/http.log" 2>&1 &
srv=$!
cleanup() { "$bin/hwatu" quit >/dev/null 2>&1 || true; kill "$srv" 2>/dev/null || true; rm -rf "$work"; }
trap cleanup EXIT
sleep 0.5
"$bin/hwatu" ping >/dev/null
url=http://127.0.0.1:8643/resp.html
"$bin/hwatu" check "$url" >/dev/null # warm the pool
echo "--- sweep (1 window, resize-reuse), 5 runs"
for _ in 1 2 3 4 5; do
  "$bin/hwatu" check "$url" --viewports 360x640,768x1024,1920x1080 --eval innerWidth --shot \
    | python3 -c 'import json,sys; j=json.load(sys.stdin); print("total_ms", j["total_ms"], "passes", [v["pass_ms"] for v in j["viewports"]])'
done
echo "--- 3 separate checks (fresh loads on the pooled window), 5 runs"
for _ in 1 2 3 4 5; do
  t0=$(date +%s%N)
  for s in 360x640 768x1024 1920x1080; do
    HWATU_HEADLESS_SIZE=$s "$bin/hwatu" check "$url" --eval innerWidth --shot >/dev/null
  done
  t1=$(date +%s%N)
  echo "3-checks total $(( (t1-t0)/1000000 )) ms"
done
