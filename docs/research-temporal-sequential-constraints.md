# Temporal and sequential constraints (X26–X33)

**Purpose.** Close the shared temporal-media gap flagged by `arch-affordance` ("sequential
affordances in multi-step rituals deserve their own constraint set beyond C8") and by the
cross-domain synthesis (temporal media — music, film, service design — listed as a coverage
gap of all nine inputs, despite the meta-observation that TIME is a design dimension).

Three deliverables:

1. A constraint set for sequential/nested affordances and multi-step rituals (arrival
   sequences, onboarding, checkout, ceremony), with pass/fail tests in the style of X1–X25.
2. Adapters testing whether existing composition constraints (X17 meter-then-deviation,
   X18 one-dominant, X8 thresholds) hold under time-based media or need temporal variants.
3. Explicit numbering (X26+) so the synthesis can absorb these without renumbering.

**Grounding sources.**

- Gaver, W. "Technology Affordances," *CHI '91*: **sequential affordances** (acting on one
  affordance reveals the next — a handle affords grasping, grasping reveals turning) and
  **nested affordances** (affordances grouped so one serves as context for another).
- Alexander et al., *A Pattern Language* (1977): Pattern 112 **Entrance Transition** (mark
  the street-to-interior passage with changes of light, sound, direction, surface, level),
  Pattern 110 Main Entrance, Pattern 127 Intimacy Gradient, Pattern 134 Zen View.
- Shostack, G.L. "Designing Services That Deliver," *HBR* 62(1), 1984: **service
  blueprinting** — line of visibility, fail points, execution-time standards; later journey
  mapping practice (stages, touchpoints, emotion curve).
- Peak-end and duration neglect: Fredrickson & Kahneman 1993; Redelmeier & Kahneman 1996;
  Chase & Dasu, "Want to Perfect Your Company's Service? Use Behavioral Science," *HBR* 2001
  (finish strong, segment pleasure / combine pain, get bad experiences out of the way early).
- First-touch findings from `arch-sensory`: Pallasmaa's door-handle-as-handshake (*The Eyes
  of the Skin*), Lindgaard et al. 2006 (visual appeal judged in ~50 ms), Maister 1985
  ("The Psychology of Waiting Lines": occupied, explained, fair waits feel shorter).
- Temporal-media theory used by the adapters: Meyer, *Emotion and Meaning in Music* (1956);
  Lerdahl & Jackendoff, *GTTM* (1983) — metrical grid + reduction hierarchy; Huron, *Sweet
  Anticipation* (2006) ITPRA; Murch, *In the Blink of an Eye* (Rule of Six — emotion over
  rhythm over spatial continuity); Cutting et al. on shot-length distributions.

**Scope of "sequence."** These constraints apply to any designed experience whose meaning
depends on order: building arrival sequences, product onboarding, checkout, restaurant
service, religious or civic ceremony, album/track ordering, film scenes, game tutorials.

---

## 1. Constraint set X26–X33

Format matches X1–X25: statement, rationale, then a **Pass** / **Fail** test pair that a
reviewer (human or automated) can apply deterministically.

### X26 — Reveal-next (sequential affordance chaining)

Completing any step must perceivably disclose the affordance of the next step, within one
attention shift and without external instruction. No step may end in a dead-end state.

*Rationale.* Gaver 1991: sequential affordances are how complex action is learned from
simple perception. A ritual that requires a docent, tooltip, or memory of a manual at a
step boundary has broken the chain.

- **Pass:** In a 5-user first-time walkthrough, at every step boundary ≥90% of users
  correctly name or perform the next available action unprompted. Every state in the flow
  diagram has ≥1 outgoing edge whose trigger is visible in that state.
- **Fail:** Any step whose exit requires knowledge not present in the scene (hidden
  gesture, unlabeled wait, "check your email" with no signpost), or any reachable state
  with zero visible exits.

### X27 — Marked thresholds (entrance transition)

Every transition between contexts of different intimacy, register, or commitment (street →
building, marketing site → app, browsing → checkout, secular → sacred, free → paid) must be
marked by a perceivable change in **at least two channels** (light, sound, surface, level,
direction; or layout, chrome, pace, form density), and the transition must have **extent**
— it occupies real time/space rather than being a single hard cut.

*Rationale.* Alexander Pattern 112: without a transition the interior reads as an extension
of the street; visitors arrive psychologically unprepared. This is the temporal projection
of X8 (thresholds), see adapter A3.

- **Pass:** For each context boundary in the blueprint, list the changed channels; ≥2 per
  boundary, and the transition has nonzero duration (an anteroom, an interstitial, a
  fade, a musical transition bar — not an instantaneous swap). Users asked "where did
  checkout begin?" locate the boundary consistently (±1 step).
- **Fail:** Any boundary marked by ≤1 channel, any commitment escalation (payment, data
  disclosure, vows) that arrives with no marked threshold, or a "threshold" of zero extent
  where preparation matters (e.g., cart → payment as an unannounced same-page mutation).

### X28 — One peak, strong end (temporal dominance)

A sequence must have exactly **one** designed global peak, and its final segment must rank
among the most positive touchpoints of the whole. Never end on the point of maximum effort
or extraction (payment, form-filling, security theater).

*Rationale.* Peak-end rule: remembered experience ≈ f(peak, end), with duration largely
neglected (Fredrickson & Kahneman 1993; Redelmeier & Kahneman 1996). Chase & Dasu 2001:
finish strong; front-load the unpleasant. This is X18 (one-dominant) projected into time —
see adapter A2.

- **Pass:** The blueprint's emotion/effort curve (X30) has one global maximum, and the
  final touchpoint's valence is in the top two of the sequence. Payment/effort steps are
  followed by a designed positive beat (confirmation craft, gift, view, benediction).
- **Fail:** Two or more competing climaxes of near-equal designed intensity; or the
  sequence terminates at its most negative/effortful touchpoint (checkout ends on the
  card form; building arrival ends at the security desk; ceremony ends in queue-to-exit).

### X29 — Calibrated first touch

The first touchpoint must be (a) crafted to the highest finish level in the sequence and
(b) **representative** of the whole — it may promise nothing the rest cannot keep.

*Rationale.* Primacy: appeal judgments form in ~50 ms (Lindgaard et al. 2006) and anchor
subsequent interpretation. Pallasmaa: the door handle is the handshake of the building
(`arch-sensory`). But first-touch polish that misrepresents the interior is a dark pattern
(bait-and-switch), so representativeness is a hard conjunct.

- **Pass:** 50 ms / 5 s exposure test on the first touchpoint (landing frame, façade,
  cover, greeting) yields the same adjective cluster users apply after full traversal.
  First-touch material/interaction finish ≥ the sequence's median finish.
- **Fail:** First touch is the roughest element of the sequence (default splash, loading
  wall, bare form); or exposure-test adjectives contradict full-traversal adjectives
  (elegant landing → cluttered product; grand lobby → dead corridors).

### X30 — Blueprint with recovery (line of visibility)

Every multi-step ritual must have an explicit blueprint: all touchpoints in order,
frontstage/backstage split at a line of visibility, execution-time standard per step, and
**every fail point paired with a designed recovery path**. Every frontstage promise must be
backed by a blueprinted backstage capability.

*Rationale.* Shostack 1984: services fail at unblueprinted fail points; what is not drawn
cannot be designed, timed, or recovered. This is also the honesty constraint in time — a
frontstage promise with no backstage support is temporal cladding-as-lie.

- **Pass:** A blueprint artifact exists; each step has a time standard; each identified
  fail point (payment decline, no-show, timeout, out-of-stock, rain at the entry court)
  has a recovery branch that itself satisfies X26–X28; no touchpoint occurs in reality
  that is absent from the blueprint (audit by shadowing one real traversal).
- **Fail:** No blueprint; any fail point whose "recovery" is an error state that dead-ends
  (violating X26); or a real traversal reveals touchpoints (hold music, third-party
  redirect, valet queue) that the blueprint omits.

### X31 — Shaped waits (duration pacing)

Waits are designed material, not gaps. Every wait must be **occupied, explained, and
bounded**: something to perceive or do, a stated reason, and a visible progress/position
signal. No silent uncertainty beyond the feedback threshold.

*Rationale.* Maister 1985: occupied, explained, fair, and certain waits feel shorter;
uncertain and unexplained waits feel longer than they are. Duration neglect (X28) means
pacing structure, not total length, drives memory — a longer well-shaped sequence beats a
shorter uncertain one.

- **Pass:** Every wait >1 s has progress indication; every wait >10 s has an explanation;
  every wait >1 min is occupied (view, seating, content, task) and bounded (position or
  estimate). Interactive feedback within ~100 ms at each touchpoint (inherits the X8-family
  feedback default; calibrate per medium).
- **Fail:** Any spinner of unexplained duration; any queue without position information;
  any hold state that is silent about whether the system is alive; waits "hidden" by
  deceptive progress bars (fabricated progress violates X30 honesty).

### X32 — Nested closure (bracket discipline)

Sub-sequences nested inside a ritual must **close before the parent resumes**, and the
parent must resume with its context restored. Nesting depth perceivable by the user must
not exceed the depth they can reliably restore (default: 2 open brackets).

*Rationale.* Gaver's nested affordances: an affordance can serve as context for another,
but the grouping must be perceptible. Interruption research (Miyata & Norman 1986) shows
resumption fails when suspended context is not re-presented. A checkout that detours into
account creation and never returns the cart, or a ceremony whose interlude never returns
to the liturgy, breaks the bracket.

- **Pass:** Model the flow as a bracket string; it balances (every entered sub-flow exits
  into its parent at the suspension point, with state visibly intact). User-visible open
  contexts ≤2 at any moment. After a forced sub-flow (auth, verification), a resumption
  cue restates where the user was.
- **Fail:** Any orphaned sub-flow (exit lands somewhere other than the suspension point,
  or with state lost); nesting depth >2 without an explicit map/breadcrumb; a sub-flow
  that silently commits the parent (detour that auto-places the order).

### X33 — Gradient monotonicity (intimacy/commitment ramp)

Along a sequence, intimacy and commitment must change **monotonically within a stage** and
step only at marked thresholds (X27). No oscillation: do not demand high commitment early,
drop to low, then demand high again; do not expose the most private register before a
shallower one.

*Rationale.* Alexander Pattern 127 (Intimacy Gradient): rooms ordered public → private;
violating the gradient makes every visit slightly wrong. In flows: asking for the credit
card before showing the product, or a ceremony that peaks in solemnity then returns to
administrivia before peaking again, violates the same gradient (and usually X28 too).

- **Pass:** Assign each step a commitment score (data disclosed, money at stake, social
  exposure, sacredness of register). The score sequence is piecewise monotone
  non-decreasing toward the peak, with increases occurring only at X27-marked thresholds,
  and any post-peak decrease is the designed release (denouement, confirmation, exit
  procession).
- **Fail:** Commitment oscillates (high–low–high) within a stage; a step demands more
  commitment than any threshold so far has signposted (card number on landing page,
  personal testimony demanded of a first-time visitor at the door).

---

## 2. Adapters: do X17 / X18 / X8 hold in time-based media?

### A1 — X17 (meter-then-deviation) → HOLDS; it is native here. Needs unit definitions, not a variant.

X17 (establish a regular meter before deviating; deviations motivated, not accidental) is
*derived from* temporal media, so it transfers back with full strength:

- **Music:** the metrical grid must be inferable before syncopation reads as syncopation
  rather than noise (Lerdahl & Jackendoff 1983; Meyer 1956 — affect arises from violation
  of an established expectation, which requires the expectation first; Huron 2006 ITPRA).
  Convention: establish within the first ~2–4 measures/phrases.
- **Film:** editing rhythm — a scene establishes a shot-length cadence, then a held or
  abruptly short shot creates emphasis (Murch: rhythm is rank 3 of the Rule of Six and
  yields only to emotion and story; Cutting et al.: shot lengths form structured, not
  random, distributions).
- **Service:** a touchpoint cadence (course pacing, check-in intervals) established early;
  a deliberate break in cadence (surprise amuse-bouche, upgrade announcement) reads as a
  gift precisely because cadence existed.

**Adapter test (X17-T operationalization, not a new number):** define the medium's meter
unit (measure, shot, touchpoint interval); a baseline cadence must be statistically
identifiable in the first ~4 units; each deviation beyond the baseline's variance must map
to a designed emphasis, and total deviation density must stay low enough that the meter
survives (deviations are read against the meter, not as a new meter — unless a marked
threshold (X27) declares a section change).

### A2 — X18 (one-dominant) → HOLDS with a nesting amendment: dominance becomes the peak, and it recurses.

In static composition, one element dominates simultaneously. In time, simultaneity is
unavailable: dominance becomes **one global peak** (this is X28). Two amendments:

1. **Dominance is positional, not just intensive.** Peak-end shows *where* intensity sits
   changes remembered value; a mid-sequence peak with weak end loses to a late peak of
   equal intensity.
2. **Dominance nests.** Each phrase/scene/stage may have a local climax, but locals must be
   subordinate to the global (Lerdahl & Jackendoff's reduction hierarchy: every span has a
   head; heads of spans are themselves subordinated up the tree). Two locals of equal
   designed intensity at the global level = X28 fail, exactly as two dominants = X18 fail.

**Adapter test:** X18 applies unchanged *within* any time-slice (a single frame, a single
screen, a single tableau); X28 is its projection *across* slices; nesting is checked by
building the reduction tree of local peaks and verifying a unique root.

### A3 — X8 (thresholds) → HOLDS with a temporal variant: boundaries become transitions with extent.

Spatial thresholds separate regions; temporal thresholds separate stages. Two properties
change in the projection, which is why X27 exists as the variant rather than reusing X8
verbatim:

1. **Extent is mandatory.** A wall can be thin; a temporal threshold cannot be a zero-width
   cut when the stages differ in commitment or register — preparation takes time
   (Alexander 112: the transition is a *place*, with its own length). Hard cuts are
   legitimate only between stages of equal register (film cuts inside a scene) — which is
   exactly when no X27 threshold is required.
2. **Marking must be multi-channel** because time-based attention is serial; a single-channel
   change is too easily missed in flow (hence the ≥2-channel rule in X27).

The X8-family *feedback* threshold (~100 ms) survives untouched as the per-touchpoint
responsiveness floor in X31.

**Adapter verdict summary:** X17 holds as-is (define units); X18 holds within slices and
projects to X28 across slices (with nesting); X8 holds for feedback and projects to X27
for stage boundaries (with extent + two channels). No existing constraint is invalidated
by temporal media; the temporal set is a projection, which supports the synthesis's
meta-claim that TIME is a dimension of the same system rather than a new system.

---

## 3. Numbering proposal for absorption

New constraints, no renumbering of X1–X25 required:

| ID | Name | Depends on / interacts with |
|-----|------|------------------------------|
| X26 | Reveal-next (sequential affordance chaining) | extends C8/affordance set; X30 recovery paths must satisfy it |
| X27 | Marked thresholds (entrance transition) | temporal variant of X8; gates X33 commitment steps |
| X28 | One peak, strong end | temporal projection of X18; consumes X30's emotion curve |
| X29 | Calibrated first touch | imports arch-sensory first-touch findings; honesty conjunct ties to material-honesty set |
| X30 | Blueprint with recovery | Shostack; prerequisite artifact for testing X26–X29, X31–X33 |
| X31 | Shaped waits | inherits X8-family 100 ms feedback default; Maister |
| X32 | Nested closure | Gaver nested affordances; bracket discipline |
| X33 | Gradient monotonicity | Alexander 127; increases only at X27 thresholds; peak/release shape shared with X28 |

Suggested synthesis wording for the absorption note: "X26–X33 are the temporal/sequential
projection of the composition set; adapters A1–A3 certify X17/X18/X8 under time-based
media, with X27 and X28 as the only required temporal variants."

**Calibration caveats (inherited-default style, matching the synthesis's existing
practice):** the 90% walkthrough rate (X26), two-channel rule (X27), top-2 end valence
(X28), 1 s / 10 s / 1 min wait ladder (X31), and depth-2 bracket cap (X32) are defensible
defaults, not measured universals; per-domain calibration required, as with the 90% / 100 ms
/ ≤4-ratio defaults already flagged in X20/X23/X25.
