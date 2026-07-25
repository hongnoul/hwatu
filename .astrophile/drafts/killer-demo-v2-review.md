# Review: killer-demo-v2.md

Reviewer stance: ruthless creative director + cold frontend-developer viewer
at 800 px, muted, no prior hwatu knowledge. Cross-checked against current
README positioning. Draft v2 is a real improvement over the v1 terminal
transcript, but three of its four scenes still fail their own acceptance
tests for a cold viewer.

## Ranked critique

### 1. Scene 2's proof doesn't prove anything (worst offender)

Two panes "freeze" at t=50%. To a cold viewer, a freeze is
indistinguishable from the video pausing or ending. The backup proof,
"two capture hashes matching / `repeat: byte-identical`", is pure insider
CI jargon flashed for under a second. The
`cubic-bezier(0.4, 0, 0.2, 1)` flash is unreadable at delivered size in a
6-second beat.

### 2. Scene 3 requires prior hwatu knowledge and its proof is invisible

- `HEADLESS SESSION · id 2` means nothing cold.
- "Materializes into the tiler" assumes the viewer knows tiling WMs.
- Worst: the acceptance test (preserved scroll position matches the
  preceding thumbnail) is a subtlety no one will notice in 6 seconds.
  Scroll position is the least legible form of state.

### 3. No premise for the first 7 seconds

The take opens on two page renders and a percentage. A cold viewer does
not know what problem this solves until they have already scrolled past.
The README heading above the GIF says "verification browser," which is
itself abstract.

### 4. Split-screen full viewports die at 800 px

GitHub renders the hero at roughly 830 px, so each pane is ~400 px wide.
Full page renders at 400 px are texture, not content. The heatmap
survives; the pages do not.

### 5. The "measurement rail grammar" is a creator conceit

A rail whose meaning changes three times in 20 s (`% match` → `t = 50%` →
`headless → live`) is a system only its designer will track. Viewers read
each scene independently.

### 6. Jargon in captions

"Flakes" (CI insider), "headless" (unexplained), and the end-frame
`13 ms windows` (why would I care about window spawn? nothing in the demo
showed it, so it violates the draft's own "no claim the picture does not
prove" rule).

### 7. Loop hostility

GitHub WebPs loop. End frame → hard cut to scene 1 will read as a glitch.

### 8. Scene 2 is overloaded

In 6 seconds: 1 s of motion + scrub + freeze + easing readout + hash
proof + caption. That is five reads; the budget is two.

## Exact edits

### Timing — rebalance to 2 / 7 / 6 / 4 / 1

| Time | Scene |
| --- | --- |
| 0:00–0:02 | NEW cold-open card, ink on paper, one line: **"Your agent says it's pixel-perfect. Make it prove it."** Loops cleanly from the end frame. |
| 0:02–0:09 | MEASURE (unchanged length) |
| 0:09–0:15 | PIN MOTION |
| 0:15–0:19 | HAND OFF |
| 0:19–0:20 | END FRAME |

### Scene 1 (MEASURE)

- Crop both panes to the same hero region (nav + headline + CTA), not
  full viewports. A 400 px crop of a headline is legible; a 400 px full
  page is not.
- Make the score a live ticking counter (`85.13 → 91.4 → 98.79`) synced
  to heatmap pixels disappearing. A number that moves is the proof; two
  static cuts are not.
- Caption stays: "Pixel diff gives the agent a number to climb." Best
  line in the draft.

### Scene 2 (PIN MOTION)

- Cut the hash / `byte-identical` proof entirely.
- Replace freeze-once with a visible scrub: the playhead drags
  0% → 50% → 80% → 50% and **both panes track it in perfect sync,
  including backwards**. Reverse motion is the one thing video playback
  cannot fake. That is the proof.
- Cut the easing string to `400 ms · ease-out` (readable) or drop it.
- Caption: ~~"Seek to the same frame. Compare motion without flakes."~~ →
  **"Scrub both pages to the exact same frame."**

### Scene 3 (HAND OFF)

- Replace scroll-position with legible state: the agent's page has text
  typed into a search field or a filled cart badge. The state chip shows
  a small live thumbnail with that state. The window appears in the
  workspace with the identical filled field. Typed text is state a viewer
  can verify in one glance.
- Badge: ~~`HEADLESS SESSION · id 2`~~ → **`AGENT'S BROWSER · invisible`**,
  flipping to **`YOURS · live`** on materialize.
- Caption: ~~"When judgment is needed, hand the exact session to a
  human."~~ → **"Stuck? The agent's invisible browser becomes your
  window. Same page, same state."**

### Rail

Keep it, but pin a constant left slot with the scene verb
(`MEASURE / PIN / HAND OFF`) in JetBrains Mono so the changing right side
has a stable anchor.

### End frame

- ~~`13 ms windows · 87 ms verification · one binary`~~ →
  **`one binary · 87 ms per check`**. 13 ms windows was never shown.
- Compose the end frame to dissolve into the cold-open card for the loop.

### Publish gate additions

- [ ] Take loops without a visible jump.
- [ ] All rail text ≥ 28 px delivered (24 px mono at 720p compressed to
      WebP is marginal).
- [ ] Cold-viewer test: someone who has never seen hwatu watches muted at
      800 px and names the three claims.

## README positioning note

The demo's story (measure → pin → hand off) matches the README's three
STOP lines well, but the current hero (`spawn-demo.svg`) sells spawn
latency, a claim the new demo drops. Decide which artifact owns the hero
slot; two competing hero demos dilute both.
