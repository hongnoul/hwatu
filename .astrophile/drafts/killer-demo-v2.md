# README hero demo v2: prove it visually

The rejected v1 was a terminal transcript. It proved that commands ran,
but made the viewer reverse-engineer what mattered. V2 is a visual
instrument panel: every scene is **action → visible consequence**, with
one sentence and one measured number. No typing footage. No unexplained
JSON. No claim the picture does not prove.

## Single job

In 20 seconds, make a frontend developer understand three things:

1. hwatu measures visual similarity instead of trusting an agent;
2. hwatu can pin motion to an exact frame;
3. a headless agent session can become the human's live window.

If a silent viewer at 800 px wide cannot name all three, the take fails.

## Visual system

- **Canvas:** 1600×900 master, rendered to 1280×720 for GitHub.
- **Palette:** ink `#101116`, paper `#F7F8FA`, measure blue `#7AA2F7`,
  mismatch red `#FF4D5E`, verified mint `#45D6A1`, time amber `#F5C451`.
- **Type:** JetBrains Mono Bold for measurements and scene labels;
  system sans for the one-line explanation. Minimum delivered size:
  24 px at 1280×720.
- **Signature:** a thin measurement rail across the bottom. It changes
  from `% match`, to `t = 50%`, to `headless → live`; the rail is the
  visual grammar connecting all scenes.
- **Restraint:** one accent per scene. No gradients, fake chrome,
  decorative grids, glowing cards, or terminal wallpaper.

## Timeline (20 seconds)

### 0:00–0:07 — MEASURE

```
┌──────────── reference ────────────┬──────────── agent build ────────────┐
│                                  │                                     │
│         same viewport            │       red heatmap overlay            │
│                                  │        fades as fixes land           │
├──────────────────────────────────┴─────────────────────────────────────┤
│  VISUAL MATCH  85.13%  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━▶  98.79%      │
└────────────────────────────────────────────────────────────────────────┘
```

- Open immediately on two real page renders, never a terminal.
- Label panes `REFERENCE` and `AGENT BUILD`.
- First beat: mismatched build + heatmap + `85.13%`.
- Two quick real checkpoint cuts. Heatmap visibly empties; score lands
  on the measured stable single-shot result (`98.79%`, or the exact
  value captured by the take).
- Caption: **"Pixel diff gives the agent a number to climb."**
- Acceptance: without captions, the viewer can see which side is wrong,
  where it is wrong, and that it improved.

### 0:07–0:13 — PIN MOTION

```
┌──────────── reference ────────────┬──────────── agent build ────────────┐
│       animated element            │       animated element               │
│          freezes                  │          freezes                     │
├──────────────────────────────────┴─────────────────────────────────────┤
│  ANIMATION TIME   0% ━━━━━━━━━━━━━●━━━━━━━━━━━━ 100%      t = 50%      │
└────────────────────────────────────────────────────────────────────────┘
```

- Both real renders move for one second.
- Timeline scrubs to 50%; both freeze on the exact middle frame.
- Flash the measured easing/duration beside the rail, not raw JSON:
  `400 ms · cubic-bezier(0.4, 0, 0.2, 1)`.
- Show two capture hashes matching, or a concise `repeat: byte-identical`.
- Caption: **"Seek to the same frame. Compare motion without flakes."**
- Acceptance: a silent viewer understands that time was controlled,
  not that animations were merely disabled.

### 0:13–0:19 — HAND OFF

```
┌──────────────────────────── developer workspace ──────────────────────┐
│  editor remains focused                    HEADLESS SESSION · 2        │
│                                                                       │
│             [same live browser materializes into the tiler]           │
├───────────────────────────────────────────────────────────────────────┤
│  SESSION STATE    headless ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━▶ live      │
└───────────────────────────────────────────────────────────────────────┘
```

- Start with the developer workspace visible and no browser window.
- Small badge reads `HEADLESS SESSION · id 2`.
- `focus` appears as a compact action label, not a typed command.
- The live page materializes into the tiler at the exact scroll position
  shown in the preceding thumbnail/state chip.
- Caption: **"When judgment is needed, hand the exact session to a human."**
- Acceptance: the browser appears without navigating or reloading, and
  the preserved scroll/state is visually obvious.

### 0:19–0:20 — END FRAME

`hwatu` · **measure → pin → hand off**

Small proof line: `13 ms windows · 87 ms verification · one binary`

## Production architecture

The shoot remains automated, but automation is separated from the
presentation:

1. `capture-v2.sh` runs real hwatu commands and emits evidence assets:
   reference/build PNGs, heatmaps, measured JSON, pinned motion frames,
   and a real hand-off recording.
2. `compose-v2.html` presents those assets as the visual instrument
   panel with a deterministic 20-second CSS timeline.
3. The existing invisible sway stage records that page at 1600×900.
4. `render-v2.sh` emits a full MP4 and 1280×720 animated WebP.
5. A contact sheet at 0/3/6/9/12/15/18/20 seconds is reviewed at 800 px
   before publication. Publishing is blocked unless every scene passes
   its acceptance test above.

This is reproducible without pretending the presentation itself is a
raw, unedited terminal session. All values and page states still come
from real hwatu output.

## Publish gate

- [ ] 20–22 seconds total; no idle beat over 700 ms.
- [ ] No terminal occupies more than 10% of any frame.
- [ ] `MEASURE`, `PIN MOTION`, `HAND OFF` legible at 800 px.
- [ ] Heatmap and score visibly change in scene 1.
- [ ] Motion visibly runs, scrubs, and freezes in scene 2.
- [ ] Browser visibly materializes with preserved state in scene 3.
- [ ] Every displayed number exists in the captured evidence manifest.
- [ ] WebP decoded by Pillow with expected frame count; MP4 decoded by
      ffprobe; both URLs return 200 after upload.
- [ ] GitHub rendered README reports image `naturalWidth=1280`, and two
      screenshots taken five seconds apart differ inside the hero bounds.
