# Page-clone convergence kit

Captures a live page from a hwatu window into a self-contained local
mirror, then verifies fidelity with `hwatu diff`. This is the pipeline
behind the "pixel-perfect copy of stripe.com" demo: it reached a
**100.0% average pixel match across 20 viewport positions** (default
tolerance 8) against the live site.

## How it works

1. **`extract.js`** (run in the source page via `hwatu eval`):
   serializes the *rendered* DOM, inlines CSSOM stylesheet text,
   freezes canvases to data URLs, pins the rendered values of every
   transitioned property inline (accordions/reveals/fades render the
   captured frame), records inner scroll positions (scroll-snap
   carousels), and emits an asset manifest.
2. **`materialize.py`**: downloads assets (including cross-origin CSS
   the page can't read, resolved against each sheet's own URL),
   rewrites URLs to local copies, re-injects canvas frames with their
   CSS boxes pinned, and appends a minimal scroll-restore script.
3. **`clone-page.sh`**: drives both against a window id.

## Method notes (what earlier iterations got wrong)

- **Pins are media-scoped, not inline.** Transition-state pins (open
  accordions, JS-set widths) bake one width's measurements. Emitting
  them as `@media (min-width: W-40) and (max-width: W+40)` rules keeps
  every other width on the site's own responsive CSS. An inline-pinned
  clone captured at 819px collapses at 1920px.
- **Sweep-prime before freezing.** Scroll the original to the bottom
  and back first so IntersectionObserver reveals and lazy content
  reach their final state; then kill timers/rAF/IO and capture.
- **Verify as a matrix, never a point.** One viewport's 100% says
  nothing about other widths. Step both windows through
  `hwatu resize` (mobile/tablet/laptop/desktop) x scroll offsets and
  read the `envelope` field of each diff: the score covers exactly
  that engine/viewport/frame and nothing else.

## Usage

```sh
hwatu --headless https://stripe.com   # window 1
hwatu wait-load
hwatu resize 1920x1080                # capture at the width you care about
# sweep-prime reveals, then freeze JS time (see method notes)
hwatu seek --progress 0               # freeze animations at t=0
TESTDIR=... ./clone-page.sh out/ 1    # capture + materialize
# serve out/ and open it in window 2, then:
hwatu seek --id 2 --progress 0
hwatu diff --id 1 --other 2 --heatmap heat.png
```

Iterate on the clone until `match_percent` converges; the heatmap and
`regions` name what to fix next. Verify motion parity with
`hwatu motion` on both windows and diff the JSON.
