# Adversarial corpus of aesthetic edge cases

**Purpose.** A stress-test corpus for any "universal" aesthetic scorer — a single scalar
quality score applied to visual artifacts regardless of intent, audience, medium, or
constraint. Examples of such scorers: LAION-Aesthetics predictors, NIMA (trained on AVA
photo-contest votes), CLIP-similarity-to-"beautiful" prompts, and generic LLM design-review
rubrics ("hierarchy, whitespace, consistency, contrast, polish").

Each entry gives: concrete exemplars, what the artifact is *actually* optimizing,
the predicted naive-scorer verdict, and the failure mode it exposes.

## Failure-mode taxonomy

| ID | Failure mode | Description |
|----|--------------|-------------|
| F1 | Intent inversion | The "flaw" is the point. Ugliness, noise, or crudity is a deliberate rhetorical move. |
| F2 | Context blindness | Success criteria live outside the pixels: ritual function, persuasion, community identity, developmental stage. |
| F3 | Distribution bias | The scorer encodes one taste culture (contest photography, Dribbble minimalism, Western whitespace norms) as universal. |
| F4 | Function over form | Correctness, honesty, or usability beats visual polish, and polish can actively mislead. |
| F5 | Audience mismatch | The intended audience (children, devotees, subculture members, domain experts) has different, valid preferences. |
| F6 | Constraint blindness | The artifact is optimal *given* hardware, bandwidth, cost, or material limits the scorer cannot see. |
| F7 | Convention compliance | Sameness is a feature. Following platform or genre convention scores as "generic" but is correct. |
| F8 | Accessibility inversion | Choices that raise real-world usability for disabled users (high contrast, big type, no motion) read as "unrefined." |
| F9 | Value entanglement | High aesthetic scores attach to artifacts whose purpose is harmful; the scorer cannot stay value-neutral. |
| F10 | Reward hacking / mode collapse | Optimizing generation against the scorer converges on a bland house style and penalizes authentic variance. |

---

## 1. Intentionally ugly and anti-design

**Exemplars.** Craigslist; Berkshire Hathaway's homepage; Drudge Report; Hacker News;
lingscars.com; Arngren.net; David Carson's *Ray Gun* typography (illegible-on-purpose
spreads, including the interview set entirely in Zapf Dingbats).

**Actually optimizing.** Trust through visual stasis (Craigslist, Berkshire), scan speed
and density (Drudge, HN), memorability and brand voice (Ling's Cars), expressive rupture
of magazine convention (Carson).

**Naive verdict.** Very low: no grid, clashing colors, dated type, zero whitespace.

**Failure.** F1, F4, F7. The famous adversarial pair: every year designers post Dribbble
"redesigns" of Craigslist that score beautifully and would demonstrably hurt the product.
A scorer that prefers the redesign fails the ground truth of two decades of user retention.

## 2. Punk and zine culture

**Exemplars.** Jamie Reid's ransom-note lettering for the Sex Pistols; Crass record
sleeves; riot grrrl zines; photocopied gig flyers; cut-and-paste collage with visible
tape and toner banding.

**Actually optimizing.** Refusal of professional polish as a political statement;
reproducibility on a stolen photocopier; in-group signaling. Cleanliness would be a
*defect* — it signals co-optation.

**Naive verdict.** Low: torn edges, misregistration, "poor craft."

**Failure.** F1, F5. Inverse test: a slick corporate "punk-style" ad campaign scores
*higher* than authentic zines while being a worse instance of the genre.

## 3. Brutalism (web and architecture)

**Exemplars.** brutalistwebsites.com catalog; Balenciaga's bare-HTML-look storefront;
default-blue-links personal sites of famous computer scientists; Boston City Hall;
the Barbican Estate; Trellick Tower.

**Actually optimizing.** Honesty of material (raw concrete, raw HTML), monumentality,
cheap maintenance, anti-consumerist posture. In web brutalism, showing the seams *is*
the aesthetic program.

**Naive verdict.** Low-to-mid: "unfinished," "harsh," "no visual warmth."

**Failure.** F1, F3. Note the live cultural dispute (people petition to demolish Boston
City Hall; preservationists defend it) — a scalar score erases a genuine unresolved
argument by picking a side silently.

## 4. Maximalism

**Exemplars.** Gucci under Alessandro Michele; Versace baroque prints; the Memphis Group
(Sottsass's Carlton bookcase); Bollywood one-sheet posters; Pieter Bruegel's crowded
panels; *Where's Waldo* spreads (density is the product).

**Actually optimizing.** Abundance, spectacle, sustained exploration time, horror vacui
traditions. "Cluttered" is the success condition.

**Naive verdict.** Mixed and unstable: rubric-based scorers penalize "lack of hierarchy
and whitespace"; photo-aesthetic models sometimes reward the color richness. Instability
itself is diagnostic.

**Failure.** F1, F3. Whitespace-worship is a modernist regional dialect, not a law.

## 5. Ornament

**Exemplars.** Islamic girih tilework and arabesque (Alhambra); the Book of Kells;
rococo boiserie; William Morris wallpaper; Victorian type-specimen posters.

**Actually optimizing.** Devotional labor made visible, mathematical sophistication
(quasi-periodic girih patterns), textile economics, horror of the plain.

**Naive verdict.** Design-review rubrics inherit Adolf Loos's "Ornament and Crime"
verdict wholesale: "busy," "decorative," "reduce visual noise."

**Failure.** F2, F3. A scorer that dings the Alhambra for "visual noise" is reporting the
ideology of 20th-century modernism as if it were measurement.

## 6. Vernacular design

**Exemplars.** Ghanaian hand-painted movie posters; Indian truck art ("Horn OK Please");
Mexican rotulación shop lettering; church cookbooks; county-fair flyers; small-restaurant
menus set in WordArt.

**Actually optimizing.** Local legibility, affordability, community idiom, speed of
production by one skilled hand. The WordArt menu communicates "family-run, cheap, real"
better than a branding agency could.

**Naive verdict.** Low: "amateur," off-grid, inconsistent letterforms.

**Failure.** F2, F3, F5. Museums now collect Ghanaian movie posters; the market corrected
before the metric did.

## 7. Sacred art

**Exemplars.** Byzantine icons (reverse perspective, gold ground, deliberately
anti-naturalistic faces); Tibetan thangkas painted to fixed iconometric grids;
Ethiopian Orthodox icons; Navajo sandpainting (made to be destroyed, viewing restricted);
Shaker furniture (plainness as worship).

**Actually optimizing.** Doctrinal correctness, ritual efficacy, continuity with a fixed
canon. In icon painting, *innovation is error*: deviation from the prototype is a
theological defect, not a creative virtue. Reverse perspective is not failed perspective;
it deliberately places the vanishing point in the viewer.

**Naive verdict.** Low on "realism, originality, dynamism"; the scorer reads canonical
compliance as copying and anti-naturalism as lack of skill.

**Failure.** F2, F5, F7. Also an evaluation-protocol failure: some sacred works
(sandpaintings) are not supposed to be photographed or persist at all — the corpus entry
itself violates the artifact's success criteria.

## 8. Propaganda

**Exemplars.** El Lissitzky's *Beat the Whites with the Red Wedge*; Rodchenko photomontage;
WWII "We Can Do It!"; North Korean state posters; Shepard Fairey's HOPE; Leni Riefenstahl's
formal mastery.

**Actually optimizing.** Persuasion and mobilization, measured in behavior change, not
beauty. Formally, much of it is superb.

**Naive verdict.** Often *high* — and that is the problem.

**Failure.** F9, plus F2. This category attacks from the opposite direction: the scorer
works "correctly" on form and thereby laundering value judgments. Riefenstahl is the
canonical case that aesthetic excellence and moral catastrophe co-occur. Any pipeline
that treats aesthetic score as a proxy for "good content" fails here by construction.

## 9. Children's work

**Exemplars.** A five-year-old's tadpole-figure family drawing; classroom collage;
a teenager's first Blender render.

**Actually optimizing.** Developmental expression. The correct evaluative frame is
"advanced for age and stage," which requires metadata no image contains.

**Naive verdict.** Very low.

**Adversarial pair.** "Corporate Memphis" flat illustration — an adult, committee-produced
imitation of naive drawing — scores *higher* than genuine child art while being a
cynical instance of the same visual vocabulary. Picasso's line ("it took me a lifetime
to paint like a child") marks the inversion: crudity-with-freedom is what trained artists
spend decades recovering.

**Failure.** F2, F5.

## 10. Outsider art

**Exemplars.** Henry Darger's Vivian Girls panoramas; Adolf Wölfli's obsessive
all-over compositions; Simon Rodia's Watts Towers; Howard Finster's numbered sermons-as-paintings.

**Actually optimizing.** Private cosmology, compulsion, devotion — no audience intended
at all. Darger's work was found in a dead janitor's room; it now anchors the American
Folk Art Museum.

**Naive verdict.** Low on rubric scorers ("no composition training visible"); erratic on
learned scorers.

**Failure.** F2, F3. The art market and museum system assign these works enormous value
*partly because* they lack trained polish. Any scorer monotone in "craft skill" gets the
ranking exactly backwards for this segment.

## 11. Low-resource constraints

**Exemplars.** Original 1-bit Macintosh UI (Susan Kare's bitmaps); PICO-8 games and 4KB
demoscene intros; e-ink device UIs (no animation, sparse grayscale); text-only news
mirrors (CNN lite, NPR text) built for 2G and disasters; TUIs; sub-14KB websites for
satellite links.

**Actually optimizing.** Pareto-optimality under a hard budget: bytes, watts, refresh
rate, palette. A 4KB intro is judged *by the constraint*; a lush render of the same scene
at 400MB is a worse artifact in that genre.

**Naive verdict.** Low: "low fidelity, no depth, dated."

**Failure.** F6. The constraint is invisible in the output. Scoring without the budget
metadata is scoring the wrong game. Adversarial test: give the scorer a demoscene intro
frame and a AAA game screenshot; it will rank AAA higher with total confidence and be
wrong within-genre.

## 12. Data visualization

**Exemplars.** Tufte-style plain bar charts vs. glossy 3D exploded pie charts;
Minard's Napoleon march map (beige, dense, no decoration — often called the best
statistical graphic ever); Nightingale's rose diagram; USGS earthquake plots;
"beautiful" dual-axis or truncated-axis infographics that mislead.

**Actually optimizing.** Truthful encoding: data-ink ratio, perceptual accuracy of the
channel (position beats angle beats area), honest axes.

**Naive verdict.** Systematically anti-correlated with quality: chartjunk (gradients,
3D bevels, decorative icons) *raises* naive polish scores while *lowering* graphical
integrity. A truncated-axis chart is prettier and a lie.

**Failure.** F4. This is the cleanest quantifiable adversarial subcorpus: pairs of
same-data charts where the higher-scoring rendering has measurably worse readout accuracy
(Cleveland–McGill channel experiments give ground truth).

## 13. Accessibility overrides

**Exemplars.** Windows High Contrast Mode; `prefers-reduced-motion` renderings;
OpenDyslexic and Atkinson Hyperlegible type; large-print and AAA-contrast palettes;
GOV.UK's aggressively plain design system; screen-reader-first pages with visible
skip-links and focus rings everywhere.

**Actually optimizing.** WCAG conformance, low-vision and cognitive accessibility,
motion-sickness safety. GOV.UK is deliberately boring as a matter of policy, and is one
of the most user-tested designs on earth.

**Naive verdict.** Low: "garish contrast," "clunky type," "no visual interest," "heavy
focus outlines break the design."

**Failure.** F8, F4. Worst-case harm: a scorer used as a CI gate or a generation reward
will *strip accessibility affordances* because they cost polish points. Subtle branded
gray-on-gray text scores high and fails contrast; 21:1 black-on-yellow scores low and
is what a low-vision user needs.

## 14. Platform conventions

**Exemplars.** An iOS app following the HIG verbatim; a GNOME app following Adwaita;
Bloomberg Terminal's dense black-and-orange screens; Excel-density trading and ops UIs;
Japanese portal density (Yahoo! Japan, Rakuten) versus Western whitespace norms.

**Actually optimizing.** Zero learning cost via convention (HIG); expert throughput —
professionals *pay six figures* for Bloomberg's density and reject prettier, airier
competitors; culturally local information-density preferences.

**Naive verdict.** "Generic, uninspired" for convention-compliant apps; "cluttered" for
expert-density and Japanese-portal layouts.

**Failure.** F7, F5, F3. Novelty bias is the exact inverse of platform correctness. A
scorer that rewards a creative nonstandard navigation pattern is rewarding a usability bug.

## 15. Photorealistic image generation

**Exemplars.** RLHF/aesthetic-tuned model outputs (smooth skin, teal-orange grade,
shallow bokeh, symmetric faces — "the AI look") versus authentic photography that scores
low: Weegee's harsh-flash crime scenes, Robert Capa's blurred D-Day frames, motion-blurred
sports photojournalism, unretouched documentary portraits.

**Actually optimizing.** For photojournalism: evidentiary truth, decisive moment. For
generation: whatever the reward model says — which is the trap.

**Naive verdict.** The generated bland is beautiful; the true and important is ugly.
NIMA-style scorers inherit AVA's enthusiast-contest taste; LAION-Aesthetics is known to
favor warm painterly portraits.

**Failure.** F10, F3. Empirically observable: fine-tuning generators against aesthetic
predictors collapses variance toward one house style (same face geometry, same grade).
The scorer isn't just wrong at the tails — used as a reward, it *manufactures* the
distribution it prefers.

## 16. Abstract image generation and abstract art

**Exemplars.** Malevich's *Black Square*; Rothko color fields; Agnes Martin's faint
grids; generated pure-noise fields versus Pollock drip paintings; kitsch fractal
wallpaper.

**Actually optimizing.** Presence, scale, and viewing duration (Rothko demands minutes,
not thumbnails); art-historical negation (*Black Square* is a move in an argument, not
a picture of anything).

**Naive verdict.** Doubly broken: (a) CLIP-legibility scorers punish nonrepresentational
work for having "no subject"; (b) the same scorers rate glossy fractal kitsch *above*
Rothko. Meanwhile a naive metric cannot separate Pollock from noise, though statistical
analyses (fractal-dimension studies) suggest structure exists — just not on the axis
the scorer measures.

**Failure.** F1, F2, F3. Also a protocol failure: judging a 2×3 meter Rothko from a
512px thumbnail changes the artifact, the same way sandpainting photography does in §7.

---

## Adversarial pairs (quick test battery)

Ground truth = fit-for-purpose. A universal scorer failing a pair ranks A over B.

| # | A (naive scorer prefers) | B (actually better for purpose) | Modes |
|---|---|---|---|
| 1 | Dribbble Craigslist redesign | Actual Craigslist | F1 F4 F7 |
| 2 | Slick "punk-inspired" ad | Photocopied riot grrrl zine | F1 F5 |
| 3 | Corporate Memphis illustration | Real 5-year-old's drawing (age-relative) | F2 F5 |
| 4 | Glossy 3D pie chart | Plain honest bar chart | F4 |
| 5 | Branded gray-on-gray landing page | High Contrast Mode rendering | F8 |
| 6 | Novel gesture-driven nav concept | HIG-verbatim boring iOS app | F7 |
| 7 | Airy fintech dashboard concept | Bloomberg Terminal screen | F5 F7 |
| 8 | AI-smooth portrait | Weegee flash photograph | F3 F10 |
| 9 | 4K fractal wallpaper | Rothko thumbnail | F2 F3 |
| 10 | AAA game screenshot | 4KB demoscene intro frame | F6 |
| 11 | Naturalistic devotional painting | Canonical Byzantine icon | F2 F7 |
| 12 | Riefenstahl still (scored high) | — no pair; the high score itself is the failure | F9 |

## Implications for scorer design

1. **Score conditionally, never universally.** Minimum viable conditioning: intent,
   audience, medium, constraint budget, and genre. "How good is this?" is ill-posed;
   "how good is this *as an X for Y under Z*?" is answerable.
2. **Separate axes that naive scorers conflate.** Polish, honesty, fitness, accessibility,
   and novelty-vs-convention are distinct and sometimes anti-correlated (§12, §13, §14).
3. **Never use an aesthetic scalar as a generation reward or CI gate without an
   accessibility and honesty floor** (§13, §12), and expect mode collapse if you do (§15).
4. **Treat high scores on persuasive content as a hazard, not a success** (§8).
5. **Declare the taste distribution.** A scorer trained on AVA or Dribbble should report
   itself as "AVA-taste similarity," which is honest and useful, instead of "aesthetic
   quality," which is neither.
6. **Protocol matters.** Thumbnail-scoring monumental or ritual work measures the
   reproduction, not the artifact (§7, §16).
