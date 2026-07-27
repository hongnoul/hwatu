# The acquaintance principle, aesthetic testimony, and faultless disagreement

**Purpose.** Companion to `research-aesthetic-edge-cases.md`. That doc asks whether a
universal aesthetic scorer can be *right*. This doc asks a prior question: even if a
harness's aesthetic verdict were perfectly calibrated, what is its epistemic status for
the user? Three literatures bear on this — the acquaintance principle, the aesthetic
testimony debate, and the semantics of taste disagreement — and each constrains how a
harness should *phrase* its outputs.

---

## 1. Wollheim's acquaintance principle

Richard Wollheim, *Art and Its Objects* (2nd ed., 1980), formulates what he calls the
Acquaintance Principle (AP):

> "Judgements of aesthetic value, unlike judgements of moral knowledge, must be based on
> first-hand experience of their object and are not, except within very narrow limits,
> transmissible from one person to another."

Two claims are packed in here:

1. **Grounding claim.** A properly formed aesthetic judgment must rest on the judge's
   own perceptual encounter with the object.
2. **Transmission claim.** Aesthetic judgment therefore cannot (or can only barely) be
   handed from one person to another by say-so, in the way "the train leaves at 9" can.

The AP is intuitively powerful: someone who declares *Guernica* a masterpiece having
never seen it, purely because a critic said so, seems to have done something wrong —
even if the critic is reliable and the belief is true. Whatever they have, it is not
aesthetic judgment.

**Budd's refinement.** Malcolm Budd ("The Acquaintance Principle," *BJA* 43(4), 2003)
argues the AP conflates two things: *knowledge of aesthetic value* and *appreciation*.
Testimony can plausibly transmit the former (you can come to know, on good authority,
that a work is fine), but it cannot transmit the latter — the experiential grasp of
*how* and *why* the work succeeds, the perception of the value in the object. What is
non-transmissible is appreciation, not the bare evaluative belief. On Budd's reading the
AP survives as a claim about appreciation, and the interesting question becomes why we
care so much about appreciation over correct belief (see Nguyen, §4).

---

## 2. Pessimism about aesthetic testimony: Hopkins and Meskin

**Hopkins' taxonomy.** Robert Hopkins ("How to Be a Pessimist about Aesthetic
Testimony," *Journal of Philosophy* 108(3), 2011) gives the debate its now-standard
structure. Everyone agrees on the datum: we are strikingly reluctant to form aesthetic
beliefs on testimony, in a way we are not for empirical matters. Pessimists say this
reluctance is appropriate; optimists say it is not. Hopkins splits pessimism in two:

- **Unavailability pessimism.** Aesthetic testimony *cannot* yield knowledge — some
  epistemic defect (e.g., that aesthetic knowledge constitutively requires experience of
  its ground) blocks transmission. This is the AP read epistemically.
- **Unusability pessimism.** Aesthetic knowledge by testimony is *available* — nothing
  epistemically special blocks it — but a further, non-epistemic norm forbids *using*
  it. Hopkins favors this: there is a norm of aesthetic practice ("form aesthetic
  judgments on first-hand acquaintance") analogous to the norm he defends for the moral
  case in "What Is Wrong with Moral Testimony?" (*PPR* 74, 2007). You could get
  knowledge from the critic; you are just not licensed to deploy it as your judgment.

The distinction matters for harness design because the two pessimisms indict different
things: unavailability indicts the *belief* a user would form from a machine verdict;
unusability permits the belief but indicts *deferring* — treating the verdict as
settling the aesthetic question.

**Meskin's unreliability pessimism.** Aaron Meskin ("Aesthetic Testimony: What Can We
Learn from Others about Beauty and Art?", *PPR* 69(1), 2004) takes a third, more
empirical line. There is nothing wrong *in principle* with aesthetic testimony, but in
practice it is a much worse evidence source than ordinary testimony: aesthetic experts
are hard to identify (no independent check like arriving-train times), taste communities
diverge, and testimony is contaminated by snobbery, prestige effects, and interested
parties (marketers, scenes, canon politics). Reduced trust is therefore *justified on
ordinary evidential grounds*, no special aesthetic norm needed. Note how directly this
maps onto scorer failure modes F3 (distribution bias) and F10 (reward hacking) in
`research-aesthetic-edge-cases.md`: a scorer trained on AVA photo-contest votes is
precisely a testifier whose "expertise" is a parochial taste culture presenting itself
as neutral.

**Robson's optimism.** Jon Robson ("Aesthetic Testimony," *Philosophy Compass* 7(1),
2012; "Norms of Belief and Norms of Assertion in Aesthetics," *Philosophers' Imprint*
2015; "Aesthetic Testimony and the Test of Time," and related papers) argues the
pessimist datum is overstated and the residue is explicable without epistemic defect.
On his social-norm diagnosis, we *do* absorb aesthetic beliefs from others constantly
(canons, syllabi, "the Alhambra is worth the detour"), and the reluctance we profess is
largely a norm governing *assertion and self-presentation* — it is socially gauche to
avow an aesthetic verdict you cannot back with acquaintance, which is different from it
being epistemically improper to believe it. For an optimist, a well-calibrated testifier
(human or machine) is a perfectly good source of aesthetic belief.

---

## 3. Nguyen: why we discount deference even when it works

C. Thi Nguyen ("Autonomy and Aesthetic Engagement," *Mind* 129(516), 2020) reframes the
whole debate. Both pessimists and optimists assume the point of aesthetic life is to end
up with *correct aesthetic beliefs* (the "belief account"). Nguyen argues the primary
value lies elsewhere: in the *activity* of appreciating — perceiving, exploring,
puzzling, arriving at one's own verdict (the "engagement account"). Deference is
discounted not because it fails to deliver true belief (it may well deliver it) but
because it *skips the valuable part*, like copying the answers into a crossword or
having someone else climb the mountain for you. The norm against aesthetic deference is
thus not epistemic at all; it protects aesthetic *autonomy*, the practice of appreciating
for oneself.

Crucially, Nguyen's account explains an asymmetry the harness should respect: we happily
accept *guidance* (a critic pointing at a feature: "watch how the bass line undercuts
the lyric") while resisting *verdicts* ("this album is great, believe me"). Guidance
feeds engagement; verdicts pre-empt it. This echoes Arnold Isenberg's classic point
("Critical Communication," *Phil. Review* 58, 1949) that the function of criticism is
not to transmit conclusions but to direct perception — the critic achieves "communication
at the level of the senses," giving a *perceptual proof* the reader must complete by
looking. Frank Sibley ("Aesthetic Concepts," 1959) similarly held that aesthetic
concepts are not condition-governed: no list of non-aesthetic features entails "graceful,"
so the critic's feature-citations are aids to seeing, not premises of an inference the
reader could accept on trust.

---

## 4. Faultless disagreement and the semantics of taste

Suppose the user looks at the artifact and disagrees with the harness. Who is wrong?
The semantics literature on predicates of personal taste ("tasty," "fun," and arguably
"beautiful," "elegant," "cluttered") offers three live positions:

- **Contextualism.** "This is elegant" expresses *elegant-by-standard-S*, where S is
  fixed by the speaker's context (roughly: elegant-to-me / elegant-by-my-community's
  standard). Clean semantics, but it notoriously **loses disagreement**: if the harness
  asserts elegant-by-S₁ and the user denies elegant-by-S₂, they talk past each other —
  no contradiction, so no real dispute. Critics (Kölbel, Lasersohn, MacFarlane) take the
  felt genuineness of taste disputes to refute simple contextualism.
- **Kölbel's relativism.** Max Kölbel ("Faultless Disagreement," *Proc. Aristotelian
  Soc.* 104, 2004) argues taste disputes are **faultless disagreements**: A asserts p, B
  asserts not-p, they genuinely disagree, yet *neither has made a mistake*, because the
  proposition's truth is relative to a perspective. Disagreement is preserved;
  fault is not.
- **Lasersohn's judge parameter.** Peter Lasersohn ("Context Dependence, Disagreement,
  and Predicates of Personal Taste," *Linguistics & Philosophy* 28, 2005) implements
  this compositionally: taste predicates carry a covert **judge** argument; in the
  default "autocentric" use the judge is the assessor, but speakers can also use them
  "exocentrically" (judging for another: "the ride is fun" said of a toddler's
  rollercoaster). Same content, truth relative to a judge — hence disagreement without
  fault.
- **MacFarlane's assessment sensitivity.** John MacFarlane (*Assessment Sensitivity:
  Relative Truth and Its Applications*, OUP 2014) radicalizes this: truth is relative to
  the **context of assessment**, not just of utterance. His distinctive evidence is
  **retraction**: if my standards change, I must retract my earlier "that was delicious,"
  which contextualism can't explain (the old claim was true by the old standard) but
  assessor relativism predicts.
- The residual realist/**absolutist** option (e.g., in the spirit of Hume's ideal
  critics, or invariantism à la Cappelen & Hawthorne's *Relativism and Monadic Truth*,
  2009) holds there is a fact of the matter and someone in a taste dispute is simply
  wrong — but then the "faultless" intuition and the edge-case corpus's F3/F5 failures
  (whose taste culture gets to be the fact?) become the absolutist's burden.

---

## 5. Harness implications

### 5.1 Is a harness verdict testimony?

A scalar score or "this design is poor" verdict is functionally an aesthetic assertion
offered for uptake — i.e., **testimony**, with three aggravating features:

1. **Questionable acquaintance.** Whether a model "experiences" the artifact is at best
   contested. Under *unavailability* pessimism, even human testimony can't transmit
   aesthetic knowledge, so machine testimony certainly can't. But note: the harness
   itself may fail the AP *as a judge* — if its verdict is a regression over other
   people's votes (LAION, AVA), it is testimony *about* testimony, a chain that never
   bottoms out in acquaintance anywhere.
2. **Meskin-grade unreliability, made worse.** Every reason Meskin gives for discounting
   human aesthetic testimony (hidden parochial standards, expert identification problem,
   interested training signals) applies to scorers with the added twist that the
   parochialism is baked in at training time and invisible at inference time (F3), and
   that optimization pressure against the scorer corrupts it (F10).
3. **Hopkins' norm applies to the user, regardless.** Even if the score is knowledge-apt,
   *unusability* pessimism says the user is not licensed to adopt it as their aesthetic
   judgment without looking. The harness cannot discharge the user's acquaintance
   obligation on their behalf.

Only on full Robson-style optimism is "trust the calibrated scorer" straightforwardly
fine — and even Robson's diagnosis (the norm is about assertion/self-presentation)
suggests a *harness that asserts verdicts confidently* violates the social norm on the
user's behalf.

### 5.2 Why refusing deference to a calibrated machine verdict is legitimate

Nguyen gives the strongest ground, and it is **independent of reliability**: even a
perfectly calibrated verdict pre-empts the engagement that is the point of aesthetic
practice. A user who says "I see the score is 2.1, but I looked, and I love it" is not
making an epistemic error to be corrected by better calibration; they are exercising
aesthetic autonomy, which the practice exists to protect. The relativist semantics adds
a second, semantic ground: if "cluttered" carries a judge parameter, the harness's
verdict is true at best relative to *its* (training-distribution) judge, and the user's
contrary verdict can be true relative to theirs — a **faultless disagreement**, not an
error rate. A harness that treats user disagreement as noise to be trained away has
taken a side in the contextualism/relativism/absolutism dispute without arguing for it.

### 5.3 What report language each position licenses

| Position | What the harness may say | What it may not say |
|---|---|---|
| Unavailability pessimism (AP, strict) | Feature reports only: measurements, detected properties, comparisons. | Any evaluative verdict offered for belief ("this is ugly/beautiful"). |
| Unusability pessimism (Hopkins) | Verdicts flagged as *not a substitute for looking*: "our model rates this low — verify by eye." | Verdicts framed as settling the question ("fails aesthetic review"). |
| Unreliability pessimism (Meskin) | Verdicts **indexed to their evidence base**: "low relative to AVA-contest taste; corpus skews F3." | Unindexed universal scores implying a neutral standard. |
| Optimism (Robson) | Calibrated verdicts with confidence intervals, as from any reliable instrument. | Overclaiming beyond measured calibration. |
| Engagement account (Nguyen) | **Guidance**: Isenberg-style pointing ("the 14 typefaces compete for hierarchy — look at the nav"). Verdict available on request. | Verdict-first UX that pre-empts the user's own look. |
| Relativism (Kölbel/Lasersohn/MacFarlane) | Judge-explicit claims: "cluttered *for a judge trained on Dribbble-style minimalism*." Exocentric framing ("your stated audience would likely find…"). | Judge-free absolutes ("this *is* bad design"). |

**Convergent design recommendation.** The positions disagree on *why*, but converge on
*what*: default to **evidence language, not verdict language**.

1. **Report features, not conclusions**, as the primary output: contrast ratios, type
   inventory, alignment deltas, density metrics, nearest-neighbor style matches. These
   are ordinary empirical testimony, untouched by the AP.
2. **Anchor any evaluative claim to a function or audience** ("body text fails WCAG AA
   at this size" rather than "typography is bad"). Function-anchored claims are where
   acquaintance worries are weakest and absolutism is most defensible.
3. **Make the judge parameter explicit** whenever a taste predicate is used: name the
   corpus, taste culture, or persona the verdict is relative to. This converts a covert
   Lasersohn judge into an overt, contestable one and turns F3 from a hidden bias into a
   disclosed scope.
4. **Treat user disagreement as a standing possibility of faultlessness**, not a labeling
   error: expose a "different judge" affordance rather than only a "report mistake" one.
5. **Sequence guidance before verdict** (Nguyen/Isenberg): point at features first,
   surface the scalar only on request or clearly subordinated, and phrase it as the
   model's judgment, not the artifact's fact: "our reviewer would call this cluttered"
   licenses uptake-as-evidence; "this is cluttered" demands deference the literature
   says users may — and perhaps should — refuse.

### 5.4 One-line summary

A harness's aesthetic verdict is testimony from a judge with no acquaintance, a hidden
judge parameter, and Meskin-grade reliability problems; the defensible product is
therefore an *evidence dossier with a disclosed perspective*, not a *verdict*, and user
refusal to defer is a feature of healthy aesthetic practice, not an error to engineer
away.

---

## Key sources

- Wollheim, R. (1980). *Art and Its Objects*, 2nd ed. (Acquaintance Principle.)
- Budd, M. (2003). "The Acquaintance Principle." *British Journal of Aesthetics* 43(4).
- Hopkins, R. (2007). "What Is Wrong with Moral Testimony?" *PPR* 74(3).
- Hopkins, R. (2011). "How to Be a Pessimist about Aesthetic Testimony." *Journal of Philosophy* 108(3).
- Meskin, A. (2004). "Aesthetic Testimony: What Can We Learn from Others about Beauty and Art?" *PPR* 69(1).
- Robson, J. (2012). "Aesthetic Testimony." *Philosophy Compass* 7(1).
- Robson, J. (2015). "Norms of Belief and Norms of Assertion in Aesthetics." *Philosophers' Imprint* 15.
- Nguyen, C. T. (2020). "Autonomy and Aesthetic Engagement." *Mind* 129(516).
- Isenberg, A. (1949). "Critical Communication." *Philosophical Review* 58(4).
- Sibley, F. (1959). "Aesthetic Concepts." *Philosophical Review* 68(4).
- Kölbel, M. (2004). "Faultless Disagreement." *Proceedings of the Aristotelian Society* 104.
- Lasersohn, P. (2005). "Context Dependence, Disagreement, and Predicates of Personal Taste." *Linguistics & Philosophy* 28.
- MacFarlane, J. (2014). *Assessment Sensitivity: Relative Truth and Its Applications*. OUP.
- Cappelen, H. & Hawthorne, J. (2009). *Relativism and Monadic Truth*. OUP.
