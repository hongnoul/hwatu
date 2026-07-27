# Constraint dry-run audit: X1-X25 executed against two real artifacts

Node: arch-constraint-dryrun. Executes the 25 cross-domain constraints from
arch-cross-domain-synthesis as an actual audit, records for each whether the
pass/fail test was executable as written, the verdict it gave, and the
rewording needed. Ends with revised text for every failed constraint and a
calibration note per numeric threshold.

## Artifacts audited

- **A (software):** the hwatu repo at HEAD (crates/ipc, crates/hwatud,
  crates/hwatu), including live execution of the installed CLI
  (`hwatu ping`, `list --json`, `close 999`, `--help`, an unknown
  subcommand, timing runs).
- **B (document):** the hwatu documentation corpus as a read artifact:
  README.md, VISION.md, docs/agents.md, docs/benchmarks.md, docs/roadmap.md.

Executability grades: **E** (ran as written, unambiguous verdict),
**E-** (ran only after supplying a missing procedure the text implies but
does not give), **NX** (not executable as written by an auditor: requires
users, longitudinal data, or a judgment with no decision rule).

## Summary table

| # | Executable as written | Verdict A (code/CLI) | Verdict B (docs) | Needs rewording |
|---|---|---|---|---|
| X1 | NX (user study) | pass (proxy) | pass (proxy) | yes |
| X2 | E- (cue inventory undefined) | pass at HEAD; installed binary had 1 | pass | yes |
| X3 | E | pass w/ note | pass | minor |
| X4 | E for SW/UI; NX for WR adapter | pass (1-41 ms) | n/a as written | yes (WR adapter) |
| X5 | NX (95% user prediction) | pass (proxy) | pass (proxy) | yes |
| X6 | E- (counting rule undefined) | fail (1 gap) | pass | yes |
| X7 | E- (cost floor undefined) | pass | pass | yes |
| X8 | NX (90% interrupt study) | pass (proxy) | pass | yes |
| X9 | E (doc check) half; "tested at floor" undefined | partial fail | partial fail | yes |
| X10 | NX (needs >=2 releases) | baseline recorded | baseline recorded | yes |
| X11 | E- (verdict rule subjective) | pass | pass | yes |
| X12 | E- (needs outsider; proxy used) | pass | pass | minor |
| X13 | E (with metric supplied) | pass (35.5% vs 8.5%/1.5%) | pass | minor |
| X14 | E | pass | pass | no |
| X15 | E- (grain inventory undefined) | pass | pass | yes |
| X16 | E- (Vitsoe test undefined for SW) | partial fail | pass | yes |
| X17 | E | pass | pass | no |
| X18 | E (squint needs non-visual analog) | pass | pass | minor |
| X19 | E | pass | pass | no |
| X20 | NX as scoped ("per component" + no doc class defined) | fail | partial pass | yes |
| X21 | E | pass | pass | minor |
| X22 | E- (record-of-record undefined) | pass w/ note | pass | yes |
| X23 | NX (Louvre test); body clause E | pass | pass | yes |
| X24 | NX (temporal "before" unverifiable retrospectively) | partial (written, unlabeled) | partial | yes |
| X25 | E- (evidence unit undefined) | pass | pass | yes |

Tally: 7 fully executable as written (X3, X13, X14, X17, X18, X19, X21),
10 executable only after the auditor invented a procedure (E-),
8 not executable as written (X1, X4-WR, X5, X8, X10, X20, X23-test, X24).
The synthesis's own suspicion was correct and slightly optimistic: the
weakest were indeed X20/X23/X25 plus every constraint whose test is a
percentage over users.

## Per-constraint findings

**X1 zero-label.** Test demands >90% success and <5s hesitation from
first-time users with labels stripped. No users were available; the numbers
cannot be produced by inspection. Proxy executed: bare `hwatu <url>` is the
primary action and is the first token of usage; README's bold headline
telegraphs the deliverable in one line. Both pass the proxy. The user-study
tier should stay, but the constraint needs an inspection tier to be runnable
at all in an audit.

**X2 false-affordance count = 0.** Countable, but "every cue suggesting an
action" has no enumeration source, so two auditors would count different
cues. Executed with a convention inventory (`--help`, `-h`, subcommands
named in usage, headings, links): grep for `unimplemented!`/`todo!()` found
0 stubs; however the *installed* binary answered `--help` with
`unknown flag "--help"` (a universal CLI cue not honored), already fixed at
HEAD (5b0a31e). Verdict: HEAD passes, deployed artifact had count 1.
Docs: every README heading delivers its section. Lesson: the constraint must
say which build/deployment is the audit subject.

**X3 no hidden primary path.** Executable as written (enumerate core tasks,
verify a visible route). All core tasks in usage. Note: 12 `HWATU_*` env
vars exist in code, only ~4 documented in docs/, but none is *required*,
so the constraint as worded passes. The required/optional distinction is
what makes this executable; keep it explicit.

**X4 silent-action count = 0.** SW/UI tiers executable and sharp:
`hwatu ping` 1 ms wall-clock with output; `close 999` returns
`hwatu: no window 999` exit 1; every CLI verb prints a result line.
Pass, with two orders of magnitude of headroom under the 100 ms ack budget.
The WR adapter ("each paragraph visibly advances the argument, a reader can
say what changed") is not executable without a reader protocol and gave no
verdict.

**X5 control-effect isomorphism.** ">95% prediction on first exposure" is a
user study. Proxy executed: the crate layout (ipc / hwatud / hwatu) mirrors
runtime topology (protocol / daemon / client) exactly, and Cargo.toml
dependency edges match the directory story. Pass. With n=5 users, 95% is
statistically meaningless anyway; see calibration.

**X6 constrain-don't-warn.** "Count 'do not X' warnings convertible to
structural constraints" is countable only after deciding what counts:
17 "do not/don't/must not" strings in the docs, but most are descriptions of
product behavior, not warnings to users. With the counting rule "imperative
warnings against actions the artifact could structurally prevent," docs
count is ~0. Live CLI found one real structural gap: an unknown subcommand
(`hwatu frobnicate`) silently became a web search and opened a window,
turning a typo into an action instead of an error. That is a wrong action
that could be made impossible. Verdict: A fails with count 1.

**X7 reversibility or forcing function.** Enumerable, but "friction
proportional to cost" has no cost floor, inviting debate about trivially
cheap irreversible actions (`close <id>` has no undo; a window is seconds
of state, so demanding mitigation is noise). Setup is explicitly reversible
(`--dry-run`, `--undo`, README:58). Pass once a cost floor exists.

**X8 marked thresholds.** "Interrupt users mid-task; >90% state where they
are" is a user study. Artifact-side proxy executed: every window carries an
explicit `mode` field (headless/background/normal) queryable via
`list --json`; agent-environment detection defaults to headless and is
documented. Pass. The artifact-side check ("every transition emits a
perceivable or queryable marker") was fully executable and should be the
primary test.

**X9 capability-floor testing.** Half executable: "floor persona
documented" is a doc check, and the repo passes it
(docs/research-accessibility-capability-floor.md, commit c2157bb).
"Verified at the floor" gave no verdict because nothing defines what a
floor test run is. Both artifacts: persona documented, floor verification
not evidenced. Partial fail.

**X10 signage-as-defect metric.** "Count per release; must trend down" is
unexecutable on a single snapshot: a trend needs >=2 releases of ledger.
Executed the countable half and recorded the baseline: 1 workaround
comment in ~14.5k lines of Rust (verify.rs:195, cause cited), tooltip-class
explanatory patches in docs ~0. The constraint must say the first audit
establishes a baseline and passes vacuously.

**X11 Billington test.** "Name the concrete cost it saves now" is
executable; "pays for itself" is not a decision rule. Sampled: the separate
ipc crate has two real consumers today; the prewarm pool buys measured
13-16 ms spawns (benchmarks.md). Both name current, not speculative,
beneficiaries: pass. Rule needed: justification citing only future
flexibility = fail.

**X12 load-path legibility.** Needs an outsider; auditor-as-outsider proxy
used (structure drawn from directory and file names only, then checked
against Cargo.toml and module imports; they match). Docs: README's
Documents section is an accurate load map of the corpus. Pass. Minor
reword: permit the auditor-proxy explicitly, forbid it for the artifact's
own author.

**X13 invest at the joints.** Executable once a metric is chosen; the text
almost gives one ("compare polish at boundaries vs interiors"). Metric used:
doc-comment density. Boundary crate hwatu-ipc: 35.5% doc lines; interior
hwatud: 8.5%; hwatu CLI: 1.5%. Boundaries win decisively. Pass. Bake the
metric into the text.

**X14 substrate/frame separation.** Fully executable: "the section drawing
exists showing the two layers." VISION.md contains a literal
portable-vs-native layer table (VISION.md:55-61) naming which layers persist
and which churn. Pass both. No rewording needed.

**X15 grain check.** "List per-component grain violations" is countable
only against a named grain inventory. Using Rust idiom conventions (serde,
XDG paths, snake_case wire commands) as the inventory: no violations in
sampled files. Docs are written for skimming (tables, bold leads). Pass.
The constraint must require naming the idiom checklist per medium,
otherwise "what the medium refuses" is a vibe.

**X16 module as contract.** Two problems. The Vitsoe test ("does a new part
fit the oldest installation?") is undefined for SW: no "oldest
installation" is identified, and hwatu-ipc has no wire-level
PROTOCOL_VERSION constant or written compat policy, so the test could not
be run; that absence is itself the finding (partial fail for A). The <=4
ratio cap is a visual-scale rule with no SW/WR meaning. Docs pass: one
frozen manual-config entry, stable terminology. Crate version 0.6.0 is
pinned identically across all three consumers (Cargo.toml).

**X17 meter then deviation.** Fully executable. README STOP-list: 4
parallel repeats, no break. Wire commands uniformly snake_case. The single
deviation found (the one workaround) cites its cause inline. Pass both.
The >=3 threshold was usable exactly as written.

**X18 one dominant per level.** Executable; "squint" needs a non-visual
analog but the intent transfers: one H1 per doc, one bold headline claim in
README, one main.rs entry per binary crate. Pass. (Naive `grep -c "^# "`
overcounts by matching code-block comments; the operational check must
parse markdown structure, not lines.)

**X19 design the voids.** Fully executable. roadmap.md has a literal
"Non-goals, restated" section (roadmap.md:373); agents.md declares "It is
not a scraping browser" and names the alternative. Pass both as written.

**X20 ten-year test.** Not executable as scoped. "Aging story documented
per component" fails on granularity (per component = dozens of stories no
one will write) and never says which documents count. Searched for a
deprecation policy, dependency-rot plan, succession note: none exist, so A
fails, but the fail is only meaningful after the doc class is defined.
B partially passes on the one sub-check that IS crisp: perishable claims
date-stamped (benchmarks.md stamps every number: "Measured 2026-07-19,
remeasured 2026-07-25"), though README's own perf claims are unstamped.
The ten-year horizon is rhetoric; the checkable objects are policies and
stamps.

**X21 wear maps use.** Fully executable: audit for simulated age/activity,
count = 0. benchmarks.md states "Every number below was measured on a real
run, not estimated," names the rig, dates the runs, and links rerunnable
scripts; badges point to live CI. No forged biography found in either
artifact. Pass. Minor reword: the WR adapter "no unearned authority
signals" should become "every quantitative claim traces to a reproducible
measurement or a citation."

**X22 legible repair.** Executable only after choosing the record-of-record.
There is no CHANGELOG file; if git history is the record, it passes cleanly
(descriptive fix commits, e.g. 5b0a31e; benchmark corrections annotated
in-place: "remeasured after the composite-check work"). The constraint must
name what counts as the ledger or two auditors disagree.

**X23 stated context relation.** The body clause is executable and strong;
the given test is not. "Does it make its surroundings read better?" (Louvre
test) is an aesthetic judgment with no decision rule; it produced no
verdict. The declaration check produced a clear one: README has a whole
section "Why not Playwright or chrome-devtools-mcp?" stating relation to
prior art; agents.md states what hwatu is not and names the alternative for
the excluded use case; VISION commits to OS-native engines and says why.
Pass, among the strongest results in the audit. Swap test and aspiration.

**X24 positional decisions first.** "The list exists before detailed work
starts" is temporally unverifiable on an existing artifact: an auditor
cannot observe "before." Current-state check executed instead: the
orientation-class decisions (one protocol, native engines, warm daemon,
small core/hard boundaries) ARE written down as a product constitution
(VISION.md), but nothing labels which are irreversible. Partial. The
constraint needs two modes: gate mode for new work, audit mode for existing
artifacts.

**X25 perceptual verification pass.** Executable once the evidence unit is
defined; the text ("a documented correction pass exists; corrections are
recorded") gestures at it without defining it. Evidence found for A: the
benchmark discipline is exactly a perception-vs-system loop, with
corrections recorded and dated ("Remeasured 2026-07-22 after..."), and the
repo's own product IS a perceptual verification pass for UI work (match
percent vs claimed pixel-perfection). No capability-floor usability
evidence (consistent with X9). Pass with the SW adapter, unverdicted for
the UI-with-users tier.

## Revised constraint text (only constraints that failed executability)

Two-tier pattern used throughout: **Tier 1 (inspection)** is what one
auditor can run today and is the pass/fail test. **Tier 2 (empirical)** is
the user-study version, optional, and overrides Tier 1 when run.

- **X1'.** Tier 1: strip labels (or read only the artifact's first visible
  surface); the auditor, acting as a first-time user, must identify and
  execute the primary action using only conventions of the medium, without
  opening documentation. Tier 2: n>=5 first-time users; pass = >=4/5
  succeed unaided with time-to-first-correct-action <5 s each.
- **X2'.** Enumerate cues from a written convention inventory for the
  medium (CLI: -h/--help/--version/verbs named in usage; docs: headings,
  links, promises in the lede; UI: hover/cursor/affordance styling).
  Audit the shipped artifact, not HEAD. Count of cues without a working
  action must be 0.
- **X4'.** Scope: actions with a perceptual channel. UI ack <100 ms,
  resolution or progress <1 s. CLI/API: every invocation emits an
  action-specific result or error before exit; silent success allowed only
  with an explicit quiet flag. WR adapter (replacement): a skim-reader
  given only the first sentence of each paragraph can reconstruct the
  argument's steps; any paragraph whose first sentence adds no step is a
  countable violation.
- **X5'.** Tier 1: an auditor who has not seen the internals draws the
  control->effect (or module->runtime) map from surface arrangement alone;
  pass = drawn map matches actual with no crossing corrections on primary
  paths. Tier 2: n>=20 users, >=19/20 correct first-exposure predictions.
- **X6'.** Count only imperative warnings addressed to the user or
  operator against actions the artifact could structurally prevent
  (type-level, disabled control, rejected input). Each is a defect; also
  count wrong-action acceptances found by probing invalid input. Target 0.
- **X7'.** Irreversible = destroys state worth more than ~5 minutes of
  user work or any non-recreatable data. Enumerate; each needs undo,
  dry-run, or confirmation friction. Below the floor, no mitigation
  required.
- **X8'.** Tier 1: enumerate context/mode transitions; each must emit a
  marker that is perceivable in the moment or queryable after the fact
  (banner, prompt change, mode field, edited-state indicator). Tier 2:
  interrupt study, >=4/5 correctly locate themselves.
- **X9'.** (a) Floor persona documented; (b) at least one dated record of
  the artifact exercised under floor conditions (assistive tech session,
  zero-context getting-started run by a genuine newcomer, smallest-hand /
  largest-body fixture). Both required; (a) alone is half a pass.
- **X10'.** Keep a signage ledger (count of explanatory patches:
  trap-explaining comments, why-did-it-do-that FAQ entries, clarifying
  tooltips) per release. First audit records the baseline and passes.
  Thereafter pass = count <= previous release.
- **X11'.** For each abstraction/ornament/coordination layer: name a
  current beneficiary and the concrete cost it saves today, in writing.
  Justifications citing only future flexibility or hypothetical reuse fail.
- **X16'.** Media-split. UI/typography: one base module, closed token set
  <=4 ratios, tokens versioned. SW/ORG/WR: one named, versioned interface
  (wire constant, semver line, glossary); changes within a major are
  additive-only; a written compatibility statement identifies the oldest
  supported consumer (this names the "oldest installation" the Vitsoe test
  needs). Absence of the versioned interface or the compat statement is
  the failure.
- **X20' (aging dossier).** Replace "aging story per component" with four
  concrete objects, checked for existence and dating: (1) a deprecation or
  sunset policy for public interfaces; (2) a dependency-update/rot policy;
  (3) date-stamps on every perishable claim (benchmarks, versions,
  screenshots), evergreen content unmarked by default; (4) for UI: one
  recorded test with real accumulated data at >=10x current median volume.
  Ten years stays as the framing question that generates the dossier, not
  as a measurement.
- **X22'.** The artifact names its record-of-record (CHANGELOG, git
  history with descriptive messages, edit indicators). Every fix appears
  there; history rewrites of published record fail; in-place corrections
  annotated at the site of correction.
- **X23'.** Test: the artifact contains an explicit written statement of
  its relation to named neighbors (prior art, platform conventions,
  adjacent tools), and every deliberate convention break cites its reason.
  Absence of any stated relation = fail (indifference). The Louvre
  question ("does it make its context read better?") is retained as a
  prompt for the statement's content, not as the pass/fail test.
- **X24'.** Gate mode (new work): the irreversible-decision list is a
  required artifact before detailed work begins. Audit mode (existing
  artifacts): orientation-class decisions are written down AND each is
  labeled with its reversibility class (one-way / two-way / one-way-after-
  adoption). Written-but-unlabeled = partial fail.
- **X25'.** Evidence unit: a dated record containing {systematic output,
  perceived or measured discrepancy, correction applied, where recorded}.
  Pass = at least one such record per release cycle touching the perceptual
  surface; corrections must be visible in the record, not silently folded
  in. SW: profiles of real workloads count. UI: floor-condition usability
  session counts.

## Calibration notes per numeric threshold

- **>90% success (X1, X8):** usability-lab convention; unreachable rigor at
  the n most teams run. At n=5, 90% cannot be distinguished from 80%.
  Recalibrated to >=4/5 of n>=5, which is what 90% operationally means at
  small n. Keep 90% only if n>=20.
- **>95% prediction (X5):** stricter than X1 with the same problem;
  meaningless below n=20. Recalibrated to >=19/20 or use the structural
  proxy tier.
- **<5s hesitation (X1):** sound; time-to-first-action is measurable from
  a recording with an unambiguous start/stop. Keep.
- **<100ms ack, <1s resolve (X4):** inherited from the classic
  response-time bands (perceived-instant ~0.1 s, uninterrupted flow ~1 s).
  Empirically discriminating and achievable: hwatu's real ack latencies ran
  1-41 ms in this audit, so the threshold neither passes everything nor
  fails everything. Keep, scoped to perceptual channels.
- **>=3 repeats (X17):** the only inherited threshold that was directly
  usable as written on both artifacts. Two instances read as coincidence,
  three as pattern; counting was unambiguous (README STOP-list = 4). Keep.
- **<=4 ratios (X16):** genuine practice norm for type/spacing scales, but
  it silently rode along into SW/WR adapters where "ratio" has no referent.
  Keep for visual token sets; elsewhere replace with "closed, enumerated,
  versioned set with a written size justification."
- **10x volume (X20 UI):** reasonable stress multiplier but undefined
  baseline; calibrated to 10x the current real median (measured, not
  guessed).
- **Ten years (X20):** not a threshold at all; a rhetorical horizon. No
  measurement uses the number. Replaced by the aging-dossier checklist;
  keep the phrase as the generative question.
- **Zero-counts (X2, X4, X6, X15, X21):** zero is the right target and is
  executable, but only relative to a defined enumeration procedure; each
  rewritten constraint now names its inventory source. An undefined-domain
  zero is unfalsifiable.

## Incidental repo findings surfaced by the audit

1. Unknown CLI subcommands fall through to URL/search handling
   (`hwatu frobnicate` opened a search window): X6 violation; a typo
   becomes an action. Candidate fix: only treat the first arg as a
   URL/search if it contains a dot, scheme, or space, else error.
2. 8 of 12 `HWATU_*` env vars are undocumented in docs/ (X3-adjacent,
   optional so not a violation, but cheap to fix).
3. hwatu-ipc has no wire-level protocol version constant or written compat
   policy (X16' fail): the crate version exists but is not visible to a
   non-Rust client on the wire.
4. README performance claims are not date-stamped (benchmarks.md's are):
   X20' item 3 inconsistency within the same corpus.
