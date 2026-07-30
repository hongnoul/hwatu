#!/usr/bin/env bash
# Scroll/render FPS benchmark for one hwatud instance.
# Usage: bench-one.sh <runtime_dir> <label>
# Talks to the daemon whose socket lives in <runtime_dir>/hwatu.sock.
# Opens the fixture page, measures idle rAF cadence and rAF cadence
# during a continuous programmatic scroll, prints one JSON line.
set -euo pipefail
RUNTIME_DIR="$1"
LABEL="$2"
FIXTURE="file://$(cd "$(dirname "$0")" && pwd)/fixture.html"
HWATU="${HWATU_BIN:-hwatu}"

run() { XDG_RUNTIME_DIR="$RUNTIME_DIR" "$HWATU" "$@"; }

out=$(run --focus "$FIXTURE" 2>&1)
id=$(echo "$out" | grep -oP 'window \K[0-9]+' | head -1)
run wait-load --id "$id" --until settled >/dev/null
sleep 1

result=$(run eval --id "$id" --timeout-ms 20000 '
(async () => {
  const stats = arr => {
    const d = arr.slice().sort((a,b)=>a-b);
    const mean = d.reduce((a,b)=>a+b,0)/d.length;
    return { fps: +(1000/mean).toFixed(1),
             median_ms: +d[Math.floor(d.length/2)].toFixed(2),
             p95_ms: +d[Math.floor(d.length*0.95)].toFixed(2),
             max_ms: +d[d.length-1].toFixed(2),
             long_frames: d.filter(x => x > 1.6*d[Math.floor(d.length/2)]).length };
  };
  const raf_deltas = async (n, work) => {
    const ts = [];
    await new Promise(res => {
      function tick(t){
        ts.push(t);
        if (work) work();
        if (ts.length < n) requestAnimationFrame(tick); else res();
      }
      requestAnimationFrame(tick);
    });
    const out = [];
    for (let i=1;i<ts.length;i++) out.push(ts[i]-ts[i-1]);
    return out;
  };
  // Warmup.
  await raf_deltas(30);
  // Idle cadence.
  const idle = await raf_deltas(240);
  // Continuous scroll: 8px per frame down, like a slow wheel drag.
  window.scrollTo(0, 0);
  const scroll = await raf_deltas(480, () => window.scrollBy(0, 12));
  window.scrollTo(0, 0);
  return JSON.stringify({ idle: stats(idle), scroll: stats(scroll) });
})()')

run close "$id" >/dev/null 2>&1 || true
# eval returns a JSON-encoded string; unwrap it.
echo "{\"label\": \"$LABEL\", \"result\": $(echo "$result" | python3 -c 'import sys,json; print(json.loads(sys.stdin.read()))')}"
