# AIUC demo: one turn, then prove it

This is the AIUC-specific README hero scenario. It complements rather than
replaces `scripts/demo/`, which remains the reproducible Stripe convergence and
motion fixture.

The recording shows a real Jcode session opening two pages headlessly, measuring
their rendered fidelity at mobile, tablet, laptop, and desktop sizes, and then
handing the same live app session to a human. AIUC currently exposes no active
Web Animations API animations, so this scenario deliberately demonstrates the
responsive verification matrix instead of pretending to exercise `motion` or
`seek`.

## Run

```sh
scripts/demo-aiuc/run.sh
```

Outputs are written to `/tmp/hwatu-demo-aiuc`:

- `demo-aiuc.mp4`
- `demo-aiuc.webp`
- `demo-aiuc-contact-sheet.png`
- `evidence/viewport-diffs.jsonl`
- `evidence/fixture-manifest.json`

The filmed agent invokes `stage-matrix.sh`, a checked-in, auditable sequence of
ordinary Hwatu commands. Keeping viewport order and settle time in that script
makes the recorded run repeatable without hiding the real tool output.

The fixture HTML is captured once per take and served from two local servers.
It remains dependent on remote Framer/font/image assets; the manifest counts
and names those dependencies. Therefore the defensible claim is rendered
fidelity for the measured WebKitGTK frames, not an offline or maintainable
reimplementation. The default gate is 99%, not 100%, because independently
loaded dynamic Framer resources can produce small frame-to-frame differences.

To use an already reviewed capture rather than fetching the live page:

```sh
AIUC_SOURCE_HTML=/path/to/reviewed/index.html scripts/demo-aiuc/run.sh
```

`--publish` is intentionally explicit because it uploads public release assets,
updates the README hero, commits, and pushes.
