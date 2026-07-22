#!/usr/bin/env bash
# clone-page.sh — capture a live page from a hwatu window into a
# self-contained local mirror, then verify with `hwatu diff`.
#
# The capture strategy: serialize the *rendered* DOM (post-JS), inline
# every same-and-cross-origin stylesheet the page actually loaded
# (via CSSOM, so CORS-blocked sheets fall back to href fetch), record
# canvas/video poster frames as data URLs, and list every external
# asset (img/src, srcset, fonts, background urls) for mirroring.
set -euo pipefail

TESTDIR="${TESTDIR:?set TESTDIR}"
H() { XDG_RUNTIME_DIR="$TESTDIR/runtime" ~/git/hwatu/target/release/hwatu "$@"; }
OUT="${1:?usage: clone-page.sh <outdir> [window-id]}"
ID="${2:-1}"
mkdir -p "$OUT/assets"

# Phase 1: extract rendered HTML + asset manifest from the live page.
H eval --id "$ID" --timeout-ms 120000 "$(cat "$(dirname "$0")/extract.js")" > "$OUT/capture.json"

# Phase 1.5: WebGL canvases yield blank toDataURL frames; capture them
# from the engine instead (screenshot while in view, crop to the
# canvas rect). Detects the device pixel ratio to crop correctly.
python3 - "$OUT" "$ID" <<'CROP'
import json, subprocess, sys, os, pathlib
out, wid = pathlib.Path(sys.argv[1]), sys.argv[2]
cap = json.load(open(out / "capture.json"))
cap = cap.get("value", cap)
blanks = [c for c in cap.get("canvases", []) if c.get("blank")]
if blanks:
    testdir = os.environ["TESTDIR"]
    hw = os.path.expanduser("~/git/hwatu/target/release/hwatu")
    env = dict(os.environ, XDG_RUNTIME_DIR=f"{testdir}/runtime")
    def H(*args, capture=True):
        return subprocess.run([hw, *args], env=env, capture_output=True, text=True).stdout.strip()
    dpr = float(json.loads(H("eval", "--id", wid, "return devicePixelRatio")) or 1)
    for c in blanks:
        # Scroll the canvas fully into view, then wait a beat for rAF draws.
        y = max(c["doc_y"] - 80, 0)
        H("eval", "--id", wid, f"scrollTo(0,{y}); return 0")
        subprocess.run(["sleep", "1.2"])
        # Isolate the canvas: hide every other element's paint
        # (visibility on ancestors is overridable by descendants, so
        # the canvas stays visible) — otherwise DOM text overlaying
        # the canvas gets baked into the crop and later double-exposes
        # behind the real text.
        iso = (
            "const st=document.createElement('style'); st.id='hwatu-iso';"
            "st.textContent='body * { visibility: hidden !important }"
            f" [data-hwatu-canvas=\"{c['i']}\"] {{ visibility: visible !important }}';"
            "document.head.appendChild(st); return 1"
        )
        H("eval", "--id", wid, iso)
        subprocess.run(["sleep", "0.3"])
        shot = out / f"canvas-shot-{c['i']}.png"
        H("shot", "--id", wid, str(shot))
        H("eval", "--id", wid, "document.getElementById('hwatu-iso')?.remove(); return 1")
        # Viewport-coord crop of the canvas rect, scaled by dpr.
        vy = c["doc_y"] - y
        crop = f'{round(c["w"]*dpr)}x{round(c["h"]*dpr)}+{round(c["doc_x"]*dpr)}+{round(vy*dpr)}'
        dest = out / "assets" / f"canvas-{c['i']}.png"
        subprocess.run(["magick", str(shot), "-crop", crop, "+repage", str(dest)], check=True)
        shot.unlink()
        print(f"canvas {c['i']}: engine crop {crop} -> {dest.name}", file=sys.stderr)
    H("eval", "--id", wid, "scrollTo(0,0); return 0")
CROP

python3 "$(dirname "$0")/materialize.py" "$OUT"
echo "clone written to $OUT"
