# Perfect clone: stripe.com from scratch, 100%

Goal: a hand-written, from-scratch clone of stripe.com's landing page
that is **pixel-perfect and motion-perfect** against the reference,
verified deterministically. This is the absolute end-goal test of the
hwatu verify loop: `diff` + `motion --observe` + `clock` + `seek`.

"From scratch" means human/agent-authored HTML/CSS/JS. The
capture/materialize pipeline (`examples/clone/`, 100% via DOM
serialization) is explicitly disqualified — it may only be used to
produce *ground truth*, never clone content.

## Why this is now possible (chick + chicken)

The previous from-scratch ceiling was ~98.6%. The residual was mostly
*temporal*, not structural: script-driven motion (the rAF marquee,
integrator-based transforms) meant reference and clone were never
photographed at the same instant, so diff charged the clone for
honest phase error.

- `hwatu clock pause|step|set` pins **every** time source (rAF,
  timers, `performance.now`, `Date.now`, CSS/WAAPI). Both windows can
  be held at the *same virtual instant*. Shots are byte-identical on
  repeat.
- `hwatu motion --observe` recovers script-driven motion as
  *parameters* (model, velocity, period, easing, r²). The marquee is
  no longer opaque: -29.99 px/s, wrap 3096px, period ~104s, r²=1.0.
  The clone can be written to spec, then verified to spec.

## Definition of success (the gate)

All of the following on a fresh daemon, fresh loads, judged by an
independent verifier agent that builds its own harness:

1. **Static parity**: `hwatu diff --other` = **100.0% at tolerance 0**
   at `clock set 0`, across the viewport matrix (6 widths x DPR 1 and
   2: 360, 768, 1024, 1280, 1528, 1920) and 5 scroll positions each.
2. **Temporal parity**: byte-identical-or-100% diffs at virtual times
   t = 0, 250, 1000, 5000, 30000, and one full marquee wrap
   (~103,960 ms), stepped on both windows in lockstep.
3. **Motion-spec parity**: `motion` declared inventory matches 32/32;
   `motion --observe` fitted models on the clone match the reference
   fits (velocity within 0.5%, period within 0.5%, r² ≥ 0.999).
4. **Repeatability**: the entire gate run twice from cold produces
   identical scores.

Reference = local mirror (`scripts/demo/fetch-reference.sh`), pinned
at a recorded fetch date. Live stripe.com drifts; the mirror is the
frozen ground truth. Record the mirror's content hash in the results.

## Known hazards (plan around these up front)

- **Cross-load nondeterminism**: two loads reach `clock pause` at
  slightly different states. Protocol: load, `clock set 0`
  immediately, treat t as *absolute virtual time since set*, never
  compare across loads without a fresh `set`.
- **`Math.random` is NOT behind the clock.** If any reference script
  uses it visibly, the clone cannot match it deterministically. Fix
  in hwatu (seedable `Math.random` in the clock shim) rather than
  working around — this is a legitimate new primitive.
- **Native-clock waits vs paused pages**: `goto --wait`, click settle
  run on real time; sequence ops so waits complete before pausing.
- **Canvas / video**: mirror already handles canvas harvest; from
  scratch means re-drawing or accepting a pinned poster state at t=0.
  Decide per element in the recon phase, document each.
- **Font rasterization**: same machine, same WebKit, fonts inlined
  (sohne.woff2 already in clone/). Should be exact; verify early with
  a text-only region diff before investing elsewhere.

## Test plan (phases, each with a numeric exit bar)

### Phase 0 — Harness
Deterministic runner script: spins isolated daemon, serves reference
(:8321) and clone (:8322), loads both, `clock set 0` on both, walks
the viewport x scroll x time matrix, emits a JSON scorecard.
**Exit**: runner produces identical scorecards on two consecutive runs
against reference-vs-reference (must be 100.0 everywhere — this
validates the harness itself before any clone work).

### Phase 1 — Recon (spec extraction)
Full inventory of the reference: section map, computed type scale,
palette, asset list, declared animation table (`motion`), observed
motion table (`motion --observe` with wrap hunt), canvas/video/
iframe census, anything using `Math.random`/network-time.
**Exit**: a `clone-spec.md` artifact a builder can implement from
without ever looking at reference source, plus the hazard list
resolved (every nondeterminism source named with a mitigation).

### Phase 2 — Static convergence at t=0
Structure → layout → type → color → assets, driven by
`diff --heatmap` worst-regions, section by section.
**Exit**: 100.0% tol 0 at t=0 for the full matrix. No motion work
allowed until this holds (motion diffs are meaningless on a
structurally wrong page).

### Phase 3 — Motion convergence
Implement each animation from the Phase 1 parameter table (declared
CSS/WAAPI copied by spec; script-driven motion re-implemented as
clean code matching fitted model). Verify by lockstep `clock step`
frame pairs and by running `motion --observe` on the clone.
**Exit**: gate criteria 2 and 3 above.

### Phase 4 — Endurance and cold gate
Wrap-period boundaries, long virtual times (clock makes minutes of
virtual time cheap), then the full Definition-of-success run, twice,
from cold, by the verifier only.
**Exit**: the gate. Publish the scorecard + method in
`scripts/demo/README.md`.

## Swarm spec

Light swarm, root coordinates, one level of fan-out. Verification is
adversarial by construction: the prover never trusts a builder claim
and re-derives every number.

| Agent | Kind | Model | Brief |
|---|---|---|---|
| `harness` | implement | gpt-5.5 (low) | Phase 0 runner + scorecard. Bar: ref-vs-ref 100.0 twice. |
| `scout` | explore | claude-fable-5 | Phase 1 recon. Deliver `clone-spec.md` artifact + hazard resolutions. May read reference source for ground truth (that's measurement, not copying). |
| `mason` | implement | gpt-5.5 (low) | Phase 2. Works only from `clone-spec.md` + diff heatmaps. One section at a time, commit per section. |
| `animator` | implement | gpt-5.5 (low) | Phase 3. Works only from the motion parameter table. |
| `prover` | verify | claude-fable-5 | Gates every phase. Builds own checks, fresh daemon, never reuses builder harnesses. Owns the final cold gate. |
| `toolsmith` | fix | claude-fable-5 | On call: when a phase is blocked by a hwatu gap (seedable Math.random, paired-clock diff convenience, etc.), fix hwatu itself, tests + commit, per repo policy. |

Sequencing: `harness` ∥ `scout` → prover gates both → `mason` →
prover gate → `animator` → prover gate → prover cold gate.
`toolsmith` spawns on demand from any blocked phase.

Rules of engagement:
- Builders never open `reference/` files directly after Phase 1;
  they see only the spec artifact and diff output. (Keeps "from
  scratch" honest.)
- Every phase report must include the scorecard JSON, not prose
  claims.
- Any score regression > 0.1% blocks merge of that section.
- Worktrees per agent (`~/git/hwatu-<agent>`), proto branches,
  prover merges.

## What "100%" is allowed to mean

Primary target: 100.0% at tolerance 0 on the frozen mirror. If a
genuinely irreproducible source survives toolsmith work (e.g. GPU
nondeterminism in a canvas), the prover may carve an explicit,
documented exclusion mask — but each mask needs a named root cause
and a tracking note. Undocumented tolerance is failure.
