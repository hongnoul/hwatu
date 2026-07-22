# Demo video: the convergence loop

Hero demo for launch. Premise: an agent recreates stripe.com's landing
page from scratch. The clone is the *pretext*; the star of the video is
the measurable verify loop — `diff` score climbing, `seek`-pinned
frames, `motion` numbers, and the human hand-off. Anyone can generate
code that looks 90% right. Only hwatu shows the agent *seeing* the last
10% and closing it.

Positioning guard: deterministic cloners (ditto.site et al.) do one-shot
scrape→code in minutes. Do NOT frame this as "watch AI clone a site" —
that fight is lost. Frame: "watch an agent converge to pixel-parity,
with numbers." The clone target just needs to be instantly recognizable
and motion-heavy; stripe.com is both.

## Shape

~90 seconds, screen capture only, no talking head. Tiling WM (niri)
visible so real windows read as real. Terminal left, browser windows
right. Timer or diff-score overlay in a corner if cheap to add.

## Beats

### 1. Cold open (0:00–0:10)

Two headless sessions, one command each:

```sh
hwatu --headless https://stripe.com          # reference
hwatu --headless http://localhost:3000       # agent's clone, current state
```

Cut straight to:

```sh
hwatu diff --id $CLONE --other $REF
{"match_pct": 87.4, "regions": [...]}
```

Caption: "87.4%. The agent can see that number. Watch it climb."

### 2. The loop, three iterations (0:10–0:50)

Each iteration is the same visual rhythm, sped up after the first:

1. `hwatu diff ... --heatmap /tmp/heat.png` — red blotches on the hero
   gradient / nav / a card grid.
2. `hwatu snapshot` + `hwatu motion` on the reference — the agent reads
   *numbers*: easing `cubic-bezier(0.215,0.61,0.355,1)`, duration
   `600ms`, the gradient animation's keyframes.
3. Agent edits code (fast cut, one or two lines on screen).
4. Re-diff: 87.4 → 93.1 → 97.8. Show the score every time. The heatmap
   visibly empties.

One iteration must be a *motion* fix, because that's undemoable
anywhere else:

```sh
hwatu seek --id $REF 0.5 && hwatu shot /tmp/ref-mid.png
hwatu seek --id $CLONE 0.5 && hwatu shot /tmp/clone-mid.png
hwatu diff ...
```

Caption: "Both animations pinned at 50%. Two shots, byte-comparable.
You cannot screenshot an animation any other way."

### 3. The hand-off (0:50–1:10)

The agent hits a judgment call it shouldn't make alone (e.g. the diff
plateaus at 98% on font antialiasing, or a region that's a rotating
testimonial). Agent runs:

```sh
hwatu focus $CLONE
```

Both windows materialize side-by-side in the WM — the *same live
sessions* the agent was driving headless. Human eyeballs them, says
"ship it" (or tweaks one thing), agent resumes headless. Caption:
"Headless is a window property. The human got the agent's exact
session, not a screenshot."

### 4. Close (1:10–1:30)

Final side-by-side, synchronized scroll on both windows
(`hwatu scroll --to-y N` on each), diff score on screen: 99.x%.

End card:

> open → verify → close: 216 ms.
> `hwatu diff`: a number your agent can climb.
> One binary. One Unix socket. `hwatu mcp` for Claude Code & Cursor.

## Production notes

- **Don't fake the loop.** Run the real agent (jcode) and record; cut
  the dead air. If an iteration flails, keep one honest retry — the
  structured errors ("2 matches, need --nth") demo well.
- **Clone starting point:** pre-generate the ~87% clone off-camera
  (any codegen; even ditto — it doesn't matter and isn't the story).
  The video opens at 87%, not 0%. From-scratch would be 40 minutes of
  boilerplate that says nothing about hwatu.
- **stripe.com is remote and changes.** Snapshot it: `wget --mirror`
  or a saved MHTML served locally, so the reference is stable across
  takes and the diff isn't polluted by their A/B tests. Also keeps the
  video reproducible for the inevitable "did you cheat" HN comment —
  publish the fixture + script.
- **Legal/optics:** recreating stripe.com's look for a demo is
  commonplace (every screenshot-to-code demo does it) but don't publish
  the clone's code in the repo; the video is enough. Alternative safe
  target if we get cold feet: linear.app (motion-heavy) or our own
  docs/index.html restyled — but recognizability sells, keep stripe
  unless there's pushback.
- **Reproducibility kit:** `scripts/demo/` with the fixture server,
  the agent prompt, and the exact hwatu commands. "Run the demo
  yourself" is the best first comment on HN.
- Every command shown must be copy-pasteable and real. No mocked JSON.

## Dependencies

- [x] `hwatu motion` / `seek` / `diff` (a35ee62)
- [x] `diff` window↔window (`--other`) and window↔baseline (`--baseline`)
      both exist — use `--other` for the loop, `--baseline` if the
      reference mirror is served as a static shot
- [ ] Synchronized scroll for the close shot (just two `scroll --to-y`
      calls; no new feature)
- [ ] Stable local mirror of stripe.com landing page
- [ ] Score/heatmap look good at video resolution (check the red-on-dim
      palette against a dark WM theme)
