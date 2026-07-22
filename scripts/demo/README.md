# hwatu demo: the convergence loop

Recreates the setup behind the demo video (script:
`.astrophile/drafts/demo-video.md`): an agent converges a hand-written
clone of stripe.com's landing page toward pixel-parity with a local
mirror, using `hwatu diff / motion / seek` as the feedback loop.

## Setup

```sh
./fetch-reference.sh                 # mirror stripe.com -> reference/ (gitignored)
python3 -m http.server 8321 --directory reference &
python3 -m http.server 8322 --directory clone &
```

## The loop

```sh
REF=$(hwatu --headless --json http://localhost:8321/ | jq .id)
CLONE=$(hwatu --headless --json http://localhost:8322/ | jq .id)
hwatu wait-load --id $REF --timeout-ms 20000

# 1. Measure: match % + worst regions + heatmap
hwatu diff --id $CLONE --other $REF --heatmap /tmp/heat.png

# 2. Read the reference's motion as numbers, not eyeballs
hwatu motion --id $REF

# 3. Compare animated states deterministically
hwatu seek --id $REF --progress 0.5
hwatu shot --id $REF /tmp/ref-mid.png     # byte-identical on repeat
hwatu seek --id $REF --resume

# 4. Edit clone/index.html, then re-measure
hwatu goto --id $CLONE http://localhost:8322/
hwatu diff --id $CLONE --other $REF

# 5. Hand off to a human at any point
hwatu focus $CLONE
```

Measured on 2026-07-22 (three manual iterations on the checked-in
clone): 79.53% → 82.86% → 85.13%. The agent's job in the video is to
keep climbing.

## Files

- `fetch-reference.sh` — mirrors stripe.com into `reference/`
  (gitignored; it's Stripe's content, refresh at will)
- `clone/index.html` — iteration-zero clone, deliberately imperfect
  (wrong type scale, off palette, missing logos, wrong easing)
- `agent-prompt.md` — the brief handed to the agent on camera

## Agent prompt

See `agent-prompt.md`; the short form:

> Reference: http://localhost:8321/. Your page: http://localhost:8322/
> (source: scripts/demo/clone/index.html). Use `hwatu diff --other` as
> your score. Climb it. Use `hwatu motion`/`seek` for animations. Call
> `hwatu focus` if you need a human judgment.
