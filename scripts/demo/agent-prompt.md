# Agent brief: converge the clone

You are improving a clone of stripe.com's landing page until it is
pixel-indistinguishable from a local mirror of the real thing.

- **Reference** (read-only): http://localhost:8321/
- **Your page**: http://localhost:8322/ — source file
  `scripts/demo/clone/index.html`. Edit only this file.

## Your feedback loop

1. `hwatu diff --id <clone> --other <ref> --heatmap /tmp/heat.png`
   returns `match_percent` and the worst mismatch regions. That number
   is your score. Every edit must raise it; if an edit lowers it,
   revert.
2. Use `hwatu snapshot` and `hwatu eval` (getComputedStyle) on the
   reference to read exact values instead of guessing: font sizes,
   colors, paddings, easings.
3. For animations, `hwatu motion --id <ref>` gives durations, delays,
   easings, and keyframes as numbers. To compare animated states, pin
   both pages with `hwatu seek --progress 0.5`, screenshot both, diff,
   then `hwatu seek --resume`.
4. After each edit: `hwatu goto --id <clone> http://localhost:8322/`
   then re-diff.

## Rules

- Report the score after every iteration.
- If the score plateaus above 97% on something that needs human
  judgment (font rendering, a rotating widget), run
  `hwatu focus <clone>` and `hwatu focus <ref>`, explain what you need
  checked, and wait.
- Static assets from the mirror may be referenced but not hotlinked
  from stripe.com.
