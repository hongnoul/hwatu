# Operationalizing X9: the capability floor

Node: `arch-accessibility-floor`. Closes the gap left by `arch-affordance` (C9: "did not
cover WCAG mappings in detail") and `arch-ergonomics-scale` ("non-Western anthropometric
datasets not checked; ANSUR II/CAESAR updates not checked"). X9 currently says "define the
capability floor" without saying how. This document says how.

**Restated X9 (operational form):** Before any aesthetic comparison, every candidate must
pass a *floor test* against a named floor persona for the artifact's declared user
population. Candidates that fail the floor are disqualified, not merely penalized. Beauty
is ranked only among floor-passing candidates. The floor is a constraint, not a score
dimension, so it cannot be traded off against elegance.

---

## 1. WCAG 2.2 success-criteria mapping across the four adapter domains

WCAG 2.2 (W3C Recommendation, 2023-10-05) adds 9 SCs over 2.1 and removes 4.1.1 Parsing
(verified against w3.org/WAI/standards-guidelines/wcag/new-in-22/). The floor level for
X9 is **all Level A + AA**, plus the specific AAA criteria named below where they are the
only testable proxy for a floor concern (reading level, focus appearance).

### 1a. UI / visual design (direct application)

| Floor concern | WCAG 2.2 SC | Executable test |
|---|---|---|
| Low vision: text legibility | 1.4.3 Contrast (Minimum) AA | ≥4.5:1 body text, ≥3:1 large text, measured with WCAG contrast formula |
| Low vision: UI parts visible | 1.4.11 Non-text Contrast AA | ≥3:1 for control boundaries, icons, focus indicators |
| Color-blind users | 1.4.1 Use of Color A | No state conveyed by hue alone; pass under deuteranopia simulation |
| 200% zoom users | 1.4.4 Resize Text AA, 1.4.10 Reflow AA | Usable at 200% zoom; no 2-D scroll at 320 CSS px width |
| Tremor / motor impairment | 2.5.8 Target Size (Minimum) AA (new in 2.2) | Every target ≥24×24 CSS px or spacing exception |
| Cannot drag | 2.5.7 Dragging Movements AA (new in 2.2) | Every drag interaction has a single-pointer alternative |
| Keyboard-only users | 2.1.1 Keyboard A, 2.4.7 Focus Visible AA, 2.4.11 Focus Not Obscured (Min) AA (new) | Full task completion by keyboard; focus never fully hidden by sticky chrome |
| Weak focus perception | 2.4.13 Focus Appearance AAA | 2 px perimeter-equivalent indicator, 3:1 change; adopt as floor for design systems even though AAA |
| Photosensitive epilepsy | 2.3.1 Three Flashes A | No content flashing >3×/s |
| Screen-reader users | 1.1.1 Non-text Content A, 1.3.1 Info and Relationships A, 4.1.2 Name/Role/Value A | Programmatic name/role/state for every control |

### 1b. Software (CLI, API, code, TUI) — WCAG mapped by analogy

WCAG formally scopes to web content; the floor concerns generalize. Mapping used by the
harness:

| Floor concern | WCAG analog | Executable test |
|---|---|---|
| Color-only signals in terminal | 1.4.1 | Output remains unambiguous with `NO_COLOR=1` / monochrome; errors also marked by text ("error:"), not only red |
| Screen-reader terminal use | 1.3.1, 1.1.1 | Progress conveyed in text lines, not only spinners/braille animation; tables have headers; no ASCII-art-only information |
| Consistent help | 3.2.6 Consistent Help A (new) | `--help`, `-h`, `help` subcommand all present, same location/format across subcommands |
| Redundant entry | 3.3.7 Redundant Entry A (new) | Config/flags persist; interactive wizards never re-ask what was already given |
| Cognitive load of auth | 3.3.8 Accessible Authentication (Min) AA (new) | Token/SSO/password-manager paths exist; no transcription of codes without copy-paste |
| Error recovery | 3.3.1 Error Identification A, 3.3.3 Error Suggestion AA | Errors name the input at fault and suggest a fix; nonzero exit codes distinguish classes |
| No forced timing | 2.2.1 Timing Adjustable A | Interactive prompts never time out, or timeout is configurable |
| Keyboard-only (TUI) | 2.1.1, 2.1.2 No Keyboard Trap | Every TUI function reachable without mouse; documented escape from every modal |

### 1c. Writing / documentation

| Floor concern | WCAG SC | Executable test |
|---|---|---|
| Reading level | 3.1.5 Reading Level AAA | Floor = lower-secondary readability for task-critical text (proxy: Flesch-Kincaid ≤ 9–10 for English; supplemental plain-language summary otherwise) |
| Jargon | 3.1.3 Unusual Words AAA, 3.1.4 Abbreviations AAA | First use of any term of art is defined or linked; abbreviations expanded once |
| Scannability | 2.4.6 Headings and Labels AA, 1.3.1 | Headings describe content; hierarchy is nested without skips; one idea per paragraph |
| Language declared | 3.1.1 Language of Page A | Docs declare language; mixed-language snippets marked |
| Instructions not sensory-only | 1.3.3 Sensory Characteristics A | Never "click the green button on the right" without a name |

### 1d. Organizational processes

WCAG maps by analogy to processes, forms, and rituals:

| Floor concern | WCAG analog | Executable test |
|---|---|---|
| Finding help/escalation | 3.2.6 Consistent Help | One documented, stable place to get help per process; same location across teams' docs |
| Re-filing the same info | 3.3.7 Redundant Entry | A person never re-enters the same data into a second internal system manually |
| Meetings exclude non-attendees | 1.2.x (captions/transcripts analog) | Every decision-bearing meeting has an async text artifact; attendance is never the only access path |
| Memory-based access | 3.3.8 | Building/system access does not require memorized codes without an alternative |
| Timed rituals | 2.2.1 | Response-time expectations (reviews, RFCs) have documented extensions; no "speak now in the meeting or lose your vote" |
| Ambiguous forms | 3.3.2 Labels or Instructions A | Internal forms label every field and state formats up front |

---

## 2. Physical floors: which anthropometric datasets, and where US numbers shift

The `arch-ergonomics-scale` node used US-centric figures. Corrections and dataset guidance:

### Datasets to use today

- **ANSUR II (2012, US Army, n≈6,068: 4,082 M / 1,986 F).** Best free, well-documented
  detailed dataset (93 measures). **Caveats: military sample** — younger, fitter, taller,
  far less obese than US civilians, excludes elderly/disabled/most extreme body sizes. Use
  for measurement *structure* and correlations, not for civilian percentile *values*,
  especially mass, waist, and any strength-adjacent proxy.
- **NHANES (CDC, continuous, civilian US).** Use to correct ANSUR II for civilian stature
  and especially **body mass** (US civilian 95th-percentile male mass materially exceeds
  ANSUR II; matters for seat structure, load ratings, dynamic loading).
- **CAESAR (1998–2000, ~4,400 civilians, US/Netherlands/Italy, 3-D scans).** Only large
  civilian 3-D surface dataset, but now 25+ years old; secular trends (obesity, slight
  stature increase) mean tail percentiles have drifted. Use shapes, not tails.
- **ISO 7250-1 (measurement definitions) + ISO/TR 7250-2 (national statistical summaries).**
  ISO/TR 7250-2 is the correct source for non-Western percentile tables (includes Japan,
  China, Korea, Thailand, Germany, US, etc.). Use it as the default cross-population source.
- **Japan: AIST anthropometric databases (1991–92 and 2004–06 "size-JPN")**, feeding JIS
  sizing standards. **China: GB/T 10000 "Human dimensions of Chinese adults"** — the 1988
  edition was the long-standing reference; a 2023 revision (GB/T 10000-2023) supersedes it
  (edition status not independently verified in this pass; see open questions).
- **DINED (TU Delft).** Free aggregation of Dutch + international datasets; Dutch data is
  the tall extreme, useful as the upper bound of the global adult envelope.

### Where US-centric numbers materially shift

- **Stature:** young-adult male means run roughly 165–172 cm in China/Japan vs ~176 cm US;
  a "5th-percentile female" global floor is set by East/Southeast Asian datasets
  (~145–150 cm), not the US ~152 cm. Overhead controls, shelf heights, and viewing heights
  sized to US P5-female exclude several extra percent of Asian users.
- **Sitting-height ratio:** East Asian populations have proportionally longer torsos /
  shorter legs (sitting-height/stature ≈ 0.53–0.55 vs ≈ 0.52 for N. Europeans). Consequence:
  US popliteal-height seat guidance (~430–480 mm) is too high; JIS-informed seating runs
  ~400–420 mm. Desk/seat pairings tuned on ANSUR fail short-legged users first.
- **Hand dimensions:** smaller grip circumference and hand length in Asian datasets shift
  tool-handle diameters and smartphone one-handed reach envelopes down.
- **Counter/worktop heights:** US kitchen convention 36 in (914 mm) vs JIS kitchen module
  850 mm (with 800/900 variants). A "beautiful" monolithic 950 mm counter fails the floor
  in Japan while passing in the Netherlands.
- **Mass and structure:** civilian US mass distributions (NHANES) exceed both ANSUR II and
  East Asian datasets by a wide margin at P95; chairs/ladders/fixtures pass different
  structural floors per market.
- **Doorway/clearance:** driven by the *large* end (Dutch P95 male ~1.94 m stature); the
  global envelope must combine the small-female floor from Asian datasets with the
  large-male ceiling from Dutch data. No single national dataset gives both.

**Rule for the harness:** a physical floor is only meaningful with a declared population.
Default global floor = P5 adult female from the smallest applicable national dataset in
ISO/TR 7250-2, combined with P95 adult male from the largest, minus exclusion zones in §4.

---

## 3. Floor-persona template (makes X9's test executable)

A floor persona is a *named bundle of minimum capabilities*, not a demographic sketch.
Template fields:

```yaml
floor_persona:
  id:                # e.g. FP-UI-1
  domain:            # software | writing | ui | org | physical
  population:        # declared user population this floor is drawn from
  sensory_floor:     # e.g. 20/70 acuity, no color discrimination, no hearing
  motor_floor:       # e.g. keyboard only; tremor: no targets <24px; grip strength ≤ 40% young-adult mean
  cognitive_floor:   # e.g. lower-secondary reading; no working-memory-dependent steps; no time pressure
  physical_floor:    # anthropometric bounds + dataset citation (only for physical artifacts)
  assistive_tech:    # what is ASSUMED present (screen reader, wheelchair, password manager)
  context:           # worst-case context (bright sun, noisy room, one-handed, bumpy bus)
  pass_tests:        # list of binary checks; ALL must pass
  disqualifies_if:   # explicit failure conditions (mirrors pass_tests, for auditability)
```

Reference personas (defaults when the artifact declares nothing):

- **FP-UI-1 "Iris":** 20/70 acuity + deuteranopia + hand tremor; browser at 200% zoom;
  keyboard-preferred. Pass tests: the 1a table rows, run as checks (contrast computation,
  target-size audit, keyboard-only task run, grayscale screenshot diff).
- **FP-SW-1 "Kass":** blind terminal user with speech output at 400 wpm; monochrome;
  no mouse. Pass tests: the 1b table rows (`NO_COLOR` run diff, help consistency check,
  no-timeout audit, error-message lint).
- **FP-WR-1 "Sam":** reads English at lower-secondary level as a second language;
  interrupted every 90 seconds. Pass tests: readability score on task-critical passages,
  jargon-definition audit, heading-hierarchy lint, sensory-instruction grep.
- **FP-ORG-1 "Dele":** remote, 9-hour timezone offset, part-time, no meeting attendance;
  anxiety around synchronous confrontation. Pass tests: decision trail exists async,
  escalation path documented in one stable place, no same-data re-entry, no
  synchronous-only decision gates.
- **FP-PH-1 "Mina":** 148 cm stature (P5 female, East Asian datasets per ISO/TR 7250-2),
  wheelchair user (see §4 reach ranges), grip strength 40% of young-adult male mean.
  Pass tests: reach-range check 380–1220 mm for essential controls, operating force
  ≤22 N (ADA operable-parts convention), clear floor space 760×1220 mm.

**X9 test procedure:** (1) artifact declares population and floor personas (or inherits
defaults above); (2) run all `pass_tests`; (3) any failure disqualifies the candidate
before aesthetic scoring; (4) the aesthetic judge never sees floor-failing candidates,
which prevents "but it's so elegant" leakage.

---

## 4. Outside the 5th–95th adult envelope: children, elderly, wheelchair users

The P5–P95 adult convention **excludes ~10% of the declared adult population by
construction and 100% of several populations by omission**:

- **Wheelchair users:** seated eye height ~1090–1160 mm; ADA unobstructed side reach
  380–1220 mm (ISO 21542 similar); knee clearance ≥685 mm under counters; standing-adult
  percentile tables say nothing about any of this. Reach and sightline floors must come
  from wheelchair datasets/standards, not from percentile trimming of standing data.
- **Children:** not scaled-down adults — proportions differ (head ratio, grip span),
  strength floors are far lower, and safety inverts some rules (a child floor includes
  *inaccessibility* requirements: guard spacing ≤99–102 mm so heads cannot pass, drug
  packaging deliberately failing the child "floor"). Use dedicated child anthropometry
  (e.g., Snyder et al. datasets; national child growth surveys), never percentile
  extrapolation.
- **Elderly:** stature loss ~2–4 cm by 75+; grip strength commonly ~50–60% of young-adult
  values; contrast sensitivity and dark adaptation degrade (design guidance commonly
  doubles required contrast/illuminance at 70+ vs 25); slower reaction times shift timing
  floors. An "adult P5 grip" from ANSUR II is roughly an elderly *median*, so
  strength-based floors drawn from working-age data silently exclude older users.

**What X9 should say (adopted wording):**

> The capability floor is defined over the artifact's *declared user population*, and the
> declaration is itself auditable: a public artifact may not declare a population that
> excludes children, elderly people, or wheelchair users merely to pass. When the
> population includes groups outside the adult P5–P95 envelope, floors are taken from
> group-specific standards and datasets (ISO 21542/ADA for wheelchair reach and clearance,
> child anthropometry for children, age-adjusted strength/vision floors for 65+), not from
> percentile extrapolation. Where fixed dimensions cannot cover the population,
> adjustability ranges replace point targets, and the floor test is run at both ends of
> the adjustment range. Safety-inverted cases (child-resistant closures) declare the
> intended exclusion explicitly so the floor auditor treats failure-for-that-group as a
> pass condition rather than a defect.

This converts X9 from an aspiration into: declared population → named floor personas →
binary pass tests → disqualification before aesthetic ranking.
