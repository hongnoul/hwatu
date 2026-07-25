# Automated demo studio

Records the README demo with no keyboard input and no windows on the real desktop. A nested headless Sway compositor runs an isolated hwatu daemon and `wf-recorder`.

## One command

```sh
scripts/demo/record/run-v2.sh
```

This captures real evidence, records the 21-second composition, renders the 1280×720 MP4 and animated WebP, builds an 800px contact sheet, and runs every local gate.

After reviewing the contact sheet, publish and verify the GitHub README with:

```sh
scripts/demo/record/run-v2.sh --publish
```

The publisher checks both asset URLs, GitHub's rendered README API, and the live GitHub DOM before it reports success.

## Story

1. **MEASURE** uses two real 60-cell responsive scorecards and captured pixel heatmaps.
2. **PIN MOTION** performs real `0 → 50 → 80 → 50%` seeks and proves the repeated 50% frames are byte-identical.
3. **HAND OFF** focuses an offscreen session and verifies its URL, scroll position, title, and typed value are unchanged.

The compact scorecards are tracked in `scripts/demo/scorecards/`. The two visual checkpoint directories remain generated under `scripts/demo/checkpoints/` and can be overridden with `HWATU_DEMO_BOOKEND_DIR` and `HWATU_DEMO_FINAL_DIR`.

## Components

```text
capture-v2.sh   real screenshots, heatmaps, motion seeks, handoff state
compose-v2.*    deterministic visual story; ?t= fixes an exact frame
render-v2.sh    headless recording, MP4/WebP/contact-sheet rendering
validate-v2.sh  fail-closed media, evidence, timing, activity, and loop gates
publish.sh      release upload, README update, and live verification
stage.sh        isolated compositor lifecycle
```

The older terminal-driven `film.sh` and `render.sh` remain available for diagnostic recordings.
