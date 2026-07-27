# Representing plural priors and contestable judgments in an aesthetic harness

**Purpose.** Companion to `research-aesthetic-edge-cases.md` (can a universal scorer be
*right*?) and `research-aesthetic-testimony.md` (what is a verdict's epistemic status?).
This doc answers the design question both leave open: if aesthetic judgment is plural
and contestable, **how should the harness represent disagreement instead of collapsing
it into a scalar?** It covers (1) philosophical grounding, (2) the ML literature on
pluralism and dissent, and (3) a concrete output-format recommendation.

---

## 1. Philosophical grounding

### 1.1 Relativism vs. contextualism (recap and the design-relevant residue)

`research-aesthetic-testimony.md` §4 lays out the semantics. The design-relevant
residue is this: **every position except naive absolutism entails that a bare scalar is
lossy in a specific, recoverable way.**

- Under **contextualism**, a score means *good-by-standard-S*; the harness that omits S
  has dropped a truth-conditionally essential parameter. Fix: name S.
- Under **Lasersohn/Kölbel relativism**, the score's truth is judge-relative and a
  contrary user verdict can be *faultless*. Fix: represent the judge, and represent
  disagreement as a first-class outcome, not as error.
- Under **MacFarlane's assessment sensitivity**, verdicts are retraction-apt when
  standards change. Fix: version the standards; a verdict is stamped with the standard
  it was assessed under and is expected to be revisable.
- Even a **Humean absolutist** gets pluralism at the practical layer, via Hume himself
  (next section).

### 1.2 Hume's true judges: an ideal that certifies *panels*, not scalars

Hume, "Of the Standard of Taste" (1757), is the classic attempt to keep a standard of
taste without pretending everyone's verdict is equal. The standard is fixed by the
**joint verdict of true judges**: those with "strong sense, united to delicate
sentiment, improved by practice, perfected by comparison, and cleared of all prejudice."

Three points matter for harness design:

1. **The standard is defined operationally through qualified judges, not through a
   property of the object.** Hume's test for delicacy is Sancho's kinsmen tasting
   leather and iron in the wine: expertise is *demonstrable sensitivity to features
   others miss*, verifiable when the key is dredged up. A harness analog: a judge
   (model, rubric, persona) earns weight by demonstrable feature-detection ability
   (does it actually notice the kerning error, the F7 convention, the F6 constraint?),
   not by fiat.
2. **"Cleared of prejudice" includes adopting the intended audience's standpoint.**
   Hume explicitly requires the critic to consider the work from the point of view of
   its intended audience ("a critic of a different age or nation... must place himself
   in the same situation as the audience"). Hume's own ideal thus mandates
   **exocentric judging** — scoring *for* the artifact's audience — which is exactly
   what F2/F5 failures in the edge-case corpus violate.
3. **Hume concedes two sources of irreducible ("blameless") variation**: the different
   humors of particular judges, and the particular manners and opinions of age and
   country. Even the strongest historical absolutism about taste ends with a residue of
   faultless plurality. Design consequence: even a maximally Humean harness should
   report a *distribution over qualified judges*, and mark which residual disagreements
   are blameless rather than resolvable.

So Hume, read carefully, does not license one scalar. He licenses a **weighted jury of
demonstrably competent, audience-situated judges, with disclosed residual variance.**

### 1.3 Standpoint epistemology: whose priors are missing?

Standpoint theory (Hartsock 1983; Harding's "strong objectivity," *Whose Science? Whose
Knowledge?* 1991; Collins, *Black Feminist Thought* 1990; Fricker, *Epistemic
Injustice*, 2007) adds two claims the semantics literature doesn't:

- **Situated knowledge / epistemic advantage.** Socially situated experience yields
  knowledge not equally available from other positions. In aesthetics this is concrete:
  the F5 audiences of the edge-case corpus (subculture members, devotees, disabled
  users, children) have *evidential* access to whether an artifact works for them that
  a distribution-majority judge lacks. A zine reader can detect co-optation polish; a
  screen-reader user can detect an F8 inversion. These are Sancho's-kinsmen
  sensitivities, unequally distributed by standpoint.
- **Epistemic injustice as a failure mode.** Fricker's *testimonial injustice*
  (systematically deflated credibility) names what a majority-vote-trained scorer does:
  minority taste cultures were outvoted at training time, so their judgments arrive
  pre-discounted (F3). Her *hermeneutical injustice* (missing interpretive resources)
  names the rubric-level version: if the harness's vocabulary has "cluttered" but not
  "barkada density" or "horror vacui as devotional plenitude," some artifacts *cannot be
  described favorably* in its terms at all.
- Harding's **strong objectivity** flips the usual worry: objectivity is *increased*,
  not decreased, by starting inquiry from marginal standpoints, because dominant
  standpoints hide their own assumptions. Design consequence: adding minority-standpoint
  judges is not a fairness tax on accuracy; it is an accuracy measure against F3.

**Grounding summary.** Four largely independent traditions — taste semantics, Hume's
ideal-critic absolutism, and standpoint epistemology — converge on the same
architecture: multiple explicitly-identified judges, weighted by demonstrable
competence, deliberately including standpoints the training majority lacks, with
residual disagreement reported as an outcome.

---

## 2. ML approaches

### 2.1 Pluralistic alignment

Sorensen et al., "A Roadmap to Pluralistic Alignment" (ICML 2024), is the standard
taxonomy. Three ways a *model* can be pluralistic:

- **Overton pluralism**: present the spectrum of reasonable responses rather than one.
- **Steerable pluralism**: faithfully adopt a specified perspective on demand.
- **Distributional pluralism**: match the response *distribution* of a population.

And three benchmark forms: multi-objective, trade-off steerable, and
**jury-pluralistic** (evaluate against a jury of raters, report per-juror agreement).
Their key empirical warning: standard RLHF *reduces* distributional pluralism —
aggregation actively collapses the very variance we want to represent. Related:
Kirk et al., PRISM Alignment Project (NeurIPS 2024 D&B), which maps preference feedback
to 1,500 participants across 75 countries and shows preference disagreement is
demographically structured, not noise.

Mapping to the harness: the verdict layer should be **Overton** (show the live
positions), the judge layer **steerable** (score *as* a named perspective), and the
calibration layer **distributional** (report where real populations split).

### 2.2 Disagreement is signal: the perspectivist turn in annotation

A decade of HCOMP/NLP work established that annotator disagreement is often not error:

- Aroyo & Welty, "Truth Is a Lie: Crowd Truth and the Seven Myths of Human Annotation"
  (*AI Magazine* 36(1), 2015): the "one truth" assumption is a myth; disagreement
  encodes item ambiguity and annotator perspective. CrowdTruth metrics quantify it.
- Plank, "The 'Problem' of Human Label Variation" (EMNLP 2022): variation is signal to
  model, not noise to adjudicate away; datasets should ship *unaggregated* labels.
- Davani, Díaz & Prabhakaran, "Dealing with Disagreements" (TACL 2022): multi-annotator
  multi-task heads that predict *each annotator's* label match or beat majority-vote
  training while preserving the ability to output uncertainty and minority views.
- Basile et al., and the SemEval-2021 Task 12 "Learning with Disagreements" shared
  task: perspectivist evaluation, scoring against soft label distributions.
- Baan et al., "Stop Measuring Calibration When Humans Disagree" (EMNLP 2022): when
  the human distribution over labels is itself split, calibrating to a majority
  hard-label is *wrong*; calibrate to the human distribution.

The aesthetic domain is the *extreme case* of all of this: taste items are precisely
those where human label variance is structural (Hume's "blameless" residue).

### 2.3 Jury learning and ensemble evaluation

- Gordon et al., "Jury Learning: Integrating Dissenting Voices into Machine Learning
  Models" (CHI 2022): instead of predicting the majority label, predict *individual
  jurors'* judgments, then let the practitioner **compose the jury** (e.g., "a jury of
  moderators including members of the targeted group") and output the jury's verdict
  *plus dissent*. This is the single closest ML precedent for the harness design below:
  who judges becomes an explicit, contestable, auditable input rather than a hidden
  training-set fact. (Its predecessor, Gordon et al.'s "The Disagreement Deconvolution,"
  CHI 2021, shows aggregate metrics badly overstate agreement with any individual.)
- LLM-as-judge ensembles: single-judge LLM evaluation has documented position, verbosity
  and self-preference biases (Zheng et al., "Judging LLM-as-a-Judge with MT-Bench and
  Chatbot Arena," NeurIPS 2023); panel/jury variants (e.g., Verga et al., "Replacing
  Judges with Juries," 2024) show ensembles of smaller diverse judges correlate better
  with humans than one large judge, at lower cost. Persona-conditioned judging is
  exactly Sorensen's steerable mode and Lasersohn's exocentric use, implemented.
- Deliberation-style prompting (multi-agent debate; also "digital juries" in content
  moderation, Fan & Zhang CHI 2020) suggests recording *why* judges split, not just that
  they did.

### 2.4 Uncertainty and dissent reporting

The key distinction is **aleatoric vs. epistemic** uncertainty (Kendall & Gal, NeurIPS
2017), which in this domain gets a third member:

| Kind | Meaning here | Correct response |
|---|---|---|
| Epistemic (model) uncertainty | The judge hasn't seen enough artifacts like this | More data/judges could resolve it; widen interval, lower confidence |
| Aleatoric (population) variance | Human taste genuinely splits on this item | **Not resolvable by more data.** Report the split itself |
| Normative contestation | The *standard* is disputed (Boston City Hall; F1/F3 items) | Report the live positions and their proponents; do not average |

Averaging the three into one interval is a category error: a 0.5 score can mean "we
don't know," "people split 50/50," or "two coherent standards disagree" — three
different downstream actions. This mirrors the ambiguity/uncertainty separation in
CrowdTruth and the "disagreement deconvolution."

### 2.5 Value-sensitive design

Friedman & Hendry, *Value Sensitive Design* (MIT Press, 2019): technology embeds values;
design for **direct and indirect stakeholders** through conceptual, empirical, and
technical investigations. Two VSD instruments port directly:

- **Stakeholder analysis → persona/judge roster.** The harness's judge set is a
  stakeholder list. VSD requires it to include indirect stakeholders (the F5 audiences,
  the F8 users) and to document who is *not* represented.
- **Value tensions are kept, not resolved.** VSD treats tensions (polish vs. honesty,
  density vs. calm, convention vs. novelty) as design material to be made visible and
  navigated per-context, which is exactly the anti-collapse stance. Related: Zhu et
  al.'s "Value-Sensitive Algorithm Design" (CSCW 2018).

---

## 3. Design recommendation: the Verdict Record

**Core principle: aggregation is a UI gesture, not a data operation.** The harness
stores and emits the full structure; any scalar shown is computed *at display time from
a named jury spec*, and the spec travels with the number.

### 3.1 Output schema

```jsonc
{
  "artifact": "landing-page@3f2c",
  "task_context": {                      // fixes the exocentric judge (Hume §1.2, F2/F5/F6)
    "declared_intent": "dense link directory, trust-through-stasis",
    "declared_audience": "returning power users",
    "constraints": ["no build step", "must render on 2G"]
  },

  // 1. Acquaintance-safe layer: measurements, no taste predicates.
  "features": [
    { "id": "contrast.body", "value": 3.9, "ref": "WCAG 1.4.3", "verdict_free": true },
    { "id": "type.families", "value": 14 },
    { "id": "style.nn", "value": "craigslist-cluster d=0.12" }
  ],

  // 2. Perspective-tagged verdicts: every taste predicate carries an overt judge.
  "verdicts": [
    {
      "judge": "minimal-web-2020s",      // resolvable id -> judge card (§3.2)
      "mode": "autocentric",
      "claim": "cluttered",
      "score": 0.22, "ci90": [0.15, 0.31],
      "grounds": ["type.families", "density.grid"],   // Isenberg-style pointers
      "standard_version": "mw-2025.2"                 // MacFarlane retraction handle
    },
    {
      "judge": "utility-density",
      "mode": "exocentric", "judging_for": "declared_audience",
      "claim": "appropriately dense",
      "score": 0.81, "ci90": [0.74, 0.88],
      "grounds": ["scanspeed.est", "style.nn"]
    }
  ],

  // 3. Disagreement block: the split is the finding.
  "disagreement": {
    "kind": "normative",                 // epistemic | population | normative (§2.4)
    "summary": "Judges split along the polish-vs-honesty tension (F1/F7).",
    "population_prior": { "split": [0.4, 0.6], "source": "PRISM-like panel, n=212" },
    "resolvable_by_more_data": false,
    "blameless": true                    // Hume's residue / Kölbel faultlessness
  },

  // 4. Contestability markers.
  "contestability": {
    "judge_roster_gaps": ["no low-vision judge run", "no non-Western-typography judge"],
    "standing": "advisory",              // advisory | gate (gates must be function-anchored)
    "dissent_channel": "add-judge | reweight-jury | dispute-grounds",
    "retraction_policy": "verdicts re-run when standard_version bumps"
  },

  // 5. Optional scalar, only ever jury-relative.
  "jury_summaries": [
    { "jury": "declared-audience-weighted", "spec": "jury/da-1.yaml",
      "score": 0.74, "dissent": 0.35, "min_judge": "minimal-web-2020s@0.22" }
  ]
}
```

### 3.2 Judge cards

Every `judge` id resolves to a **judge card** (model card for a taste standard):
provenance (corpus/persona/rubric it operationalizes), demonstrated competences (Hume's
Sancho test: which feature-detection probes it passes), known blind spots (which F1–F10
modes it is prone to), standpoint coverage, and version history. A verdict from a judge
without a card is rejected at schema level. This turns Meskin's "expert identification
problem" into an auditable artifact.

### 3.3 Behavioral rules (the anti-collapse contract)

1. **No naked scalars.** Any scalar is `(jury_spec, score, dissent, min_judge)`. The
   dissent statistic (e.g., 1 − pairwise verdict agreement) and the strongest dissenting
   judge are inseparable from the number, à la jury learning.
2. **Three uncertainties, never averaged.** CIs express *within-judge* epistemic
   uncertainty only. Population splits go in `population_prior`. Normative contestation
   is enumerated as positions, never numerically blended (see §2.4 table).
3. **Gates must be function-anchored.** Only `verdict_free` feature checks (WCAG
   thresholds, perf budgets, the capability floor of
   `research-accessibility-capability-floor.md`) may block a pipeline. Taste verdicts
   are `advisory` standing, always. This respects the acquaintance analysis
   (`research-aesthetic-testimony.md` §5.3): function-anchored claims are where
   absolutism is defensible.
4. **Guidance before verdict.** Rendering order: features → grounds → verdicts →
   jury summaries (Nguyen/Isenberg sequencing). The scalar is last and collapsed by
   default.
5. **Disagreement affordances, not error affordances.** The user-facing actions on a
   contested verdict are *add a judge*, *reweight the jury*, *dispute the grounds*
   (claim a feature reading is factually wrong), or *record faultless dissent* — not a
   thumbs-down that trains the split away. Only grounds-disputes feed accuracy training;
   recorded dissents feed the population prior. This is the schema-level enforcement of
   "user disagreement is a standing possibility of faultlessness."
6. **Roster gaps are first-class output.** Following Harding and VSD, the harness must
   say which standpoints did *not* judge. An empty `judge_roster_gaps` requires an
   attestation, not a default.
7. **Verdicts are retraction-apt.** `standard_version` on every verdict; bumping a
   judge's standard re-opens its past verdicts (MacFarlane), rather than silently
   changing what the same score means.

### 3.4 Why this rather than the two obvious alternatives

- **vs. single calibrated scalar + CI:** a CI encodes only epistemic uncertainty; it
  cannot distinguish "unknown" from "contested" (§2.4), silently picks a side in every
  F1/F3/F7 case, and per Sorensen et al., optimizing toward it destroys distributional
  pluralism (F10 mode collapse).
- **vs. free-text critique only:** loses comparability and auditability; hides the
  judge parameter *inside prose* instead of removing it; provides no dissent statistic
  to monitor for collapse over time.
  The Verdict Record keeps the machine-checkable parts (features, per-judge scores,
  dissent metrics) machine-checkable while giving contestation a typed home.

### 3.5 One-line summary

Represent aesthetic judgment as a **jury of carded, standpoint-diverse, steerable
judges over an acquaintance-safe feature layer**, with disagreement typed (epistemic vs.
population vs. normative), scalars always jury-relative and dissent-stamped, and
contestation (add-judge, reweight, dispute-grounds, faultless-dissent) built into the
schema — so the harness reports the shape of disagreement instead of laundering it into
a number.

---

## Key sources

**Philosophy.** Hume, "Of the Standard of Taste" (1757); Kölbel (2004); Lasersohn
(2005); MacFarlane, *Assessment Sensitivity* (2014); Hartsock (1983); Harding, *Whose
Science? Whose Knowledge?* (1991); Collins, *Black Feminist Thought* (1990); Fricker,
*Epistemic Injustice* (2007). See `research-aesthetic-testimony.md` for the
acquaintance/testimony sources.

**ML.** Sorensen et al., "A Roadmap to Pluralistic Alignment" (ICML 2024); Kirk et al.,
"The PRISM Alignment Project" (NeurIPS 2024); Gordon et al., "Jury Learning" (CHI 2022)
and "The Disagreement Deconvolution" (CHI 2021); Aroyo & Welty (2015); Plank (EMNLP
2022); Davani et al. (TACL 2022); Baan et al. (EMNLP 2022); Kendall & Gal (NeurIPS
2017); Zheng et al., MT-Bench (NeurIPS 2023); Verga et al., "Replacing Judges with
Juries" (2024); Fan & Zhang, "Digital Juries" (CHI 2020); Friedman & Hendry, *Value
Sensitive Design* (2019); Zhu et al. (CSCW 2018).
