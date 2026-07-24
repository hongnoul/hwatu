# clone-spec.md — stripe.com landing page, implementation spec

Phase 1 recon artifact for the perfect-clone plan
(`.astrophile/drafts/perfect-clone-plan.md`). A builder must be able
to implement the page from this file plus the raw JSON artifacts in
`scripts/demo/recon/` WITHOUT opening any file under
`scripts/demo/reference/`.

Every number here was measured on the frozen mirror served with
`python3 -m http.server 8321 --directory scripts/demo/reference`,
loaded in an isolated hwatu daemon (build a968acb, branch
proto/clone-spec, XDG_RUNTIME_DIR pointed at a scratch dir), window
resized so the page reports `innerWidth=1528, innerHeight=828,
devicePixelRatio=2` (the canonical viewport). Commands are cited
inline as `[cmd: ...]`.

Mirror content hash (sha256 of sorted per-file sha256s, first 16):
`7eb8f0a3e4fec859` — 302 files, 66 MB.
[cmd: `find reference -type f -print0 | sort -z | xargs -0 sha256sum | sha256sum`]

IMPORTANT mirror facts a builder must reproduce (the target is the
mirror as it renders locally, not live stripe.com):

- **All 35 `<img>` elements 404 in the mirror** (the mirroring script
  double-appended query strings, e.g. `...png@w=1392&fm=webp&q=60fm=webp&q=60`).
  `naturalWidth=0` for every one. They occupy layout space (their
  rects below) but paint nothing except alt text/whitespace.
  [cmd: `hwatu eval '[...document.images].map(i=>i.naturalWidth)'` → all 0;
  `hwatu console` shows HTTP 404s.]
  → The clone therefore does NOT need any raster image bytes to match
  the mirror at tolerance 0. It needs *elements with identical
  geometry that also paint nothing* (e.g. same-size `<img>` with a
  404ing src, or an empty block). Decide with the prover; the
  simplest faithful choice is an `<img>` pointing at a local 404 so
  alt-text rendering matches too. Alt text of a broken image DOES
  paint in WebKit when the image has explicit dimensions? — In this
  mirror, broken images render as blank space (no alt icon) because
  they have CSS-sized boxes. Verify early with a region diff.
- **Both WebGL canvases paint nothing** in this environment: hero
  wave canvas readback is fully transparent (0 non-transparent px of
  180 000) and the squeezy 2D canvas is all zeros at 300x150 default
  size. [cmd: eval drawImage→getImageData, count alpha>0 → 0]
- **Web fonts**: only `sohne-var` loads (inlined `@font-face`;
  `document.fonts` shows `sohne-var 1 1000 loaded`). `SourceCodePro`
  never loads (CORS-blocked from b.stripecdn.com) and no visible
  element uses it (0 elements with SourceCodePro in computed
  font-family). Builders: serve `sohne.woff2` (already in
  `scripts/demo/clone/`) as the only font. Font-feature `"ss01"` is
  set on the GDP eyebrow (`font-feature-settings: "ss01"`).

---

## 0. Page skeleton

```
<body>
  header.navigation (h=76)
  main
    section 1  hero (hero-section-container)
    section 2  modular solutions bento ("Flexible solutions...")
    section 3  sessions 2026 banner
    section 4  stats section ("The backbone of global commerce")  ← time-of-day variant
    section 5  business sizes (enterprise accordion + startups + SaaS platform)
    section 6  developers (dark, #0d1738 bg)
    section 7  what's happening (squeezy carousel + book of the week)
    section 8  ready-to-get-started CTA band
  footer (5-column sitemap)
  + two 4x4 px ThirdPartyFrame iframes appended at document end
</body>
```

Global layout: one centered `.section-container` per section,
`max-width: 1266px` (token `--hds-canary-layout-content-maxWidth` =
content maxWidth 1264px + 2px borders), with **1px solid
rgb(229,237,245) vertical border on left and right** of the container
(the faint full-height vertical guides at x=131 and x=1397 at
1528 wide). Content inside is inset 16px (`padding: ... 16px`), so
content column = x∈[148, 1380], width 1232.
[cmd: eval getComputedStyle(section-container) → border-left
`1px solid rgb(229, 237, 245)`, max-width 1266px, rect [131,·,1266,·]]

Document height at 1528x828@2x, virtual clock paused just after
load: **13932 CSS px**. (Note: 12 min after load it was 14195 because
the "What's happening" squeezy details swap heights slightly; freeze
time early. All rects below are from a fresh load paused at
virtual_ms≈660.)

## 1. Section map at 1528x828@2x

Rects are `[left, top, width, height]` in CSS px, top measured from
document origin. [cmd: `hwatu eval` getBoundingClientRect walk over
`header, main > *, footer` on a fresh paused load]

| # | section | rect | bg | key class |
|---|---------|------|----|-----------|
| 0 | header nav | [0,0,1528,76] | transparent | `navigation section--white` |
| 1 | hero | [0,76,1528,636] | #fff | `hero-section-container` |
| 2 | bento solutions | [0,712,1528,2195] | #fff | |
| 3 | sessions banner | [0,2907,1528,560] | #fff | |
| 4 | stats | [0,3467,1528,975] | #fff + radial gradient | `stats-section stats-section--time-<VARIANT>` |
| 5 | business sizes | [0,4442,1528,3979] | #fff | `business-sizes-section` |
| 6 | developers | [0,8421,1528,2338] | rgb(13,23,56) | `hds-mode--dark` |
| 7 | what's happening | [0,10759,1528,1740] | #fff | |
| 8 | ready CTA | [0,12499,1528,380] | rgb(248,250,253) | |
| 9 | footer | [0,12879,1528,1053] | rgb(248,250,253) | |

Landmarks inside sections (same coordinate system):

| landmark | rect |
|---|---|
| GDP eyebrow | [252,173,1024,21] |
| h1 (two stacked copies, see §7) | [252,234,958,220] |
| "Get started" button | [252,494,141,48] |
| "Sign up with Google" button | [401,494,212,48] |
| hero wave canvas | [381,0,1393,712] (bleeds under header) |
| hero wave fallback `<img>` | [381,-131,1392,975] (404s; paints nothing) |
| logo marquee container | [132,640,1264,72] |
| s2 h2 | [148,805,593,40] |
| first bento card (2-col) | [148,977,816,676]; others 400x676 grid, gaps 16 |
| Connect bento full-width | [148,2361,1232,450] |
| sessions banner img box | [148,2987,1232,400] |
| stats h2 | [519,3547,489,114] (centered, 2 lines) |
| stats menu strip | [132,3741,1264,182] |
| biz h2 | [148,4535,444,40] |
| enterprise accordion imgs | [148,4987..5206,1232,531] (4 stacked, 73px offsets, maxHeight-collapsed) |
| startups carousel | [148,5812,1232,623]; cards 344x448 at x=142,490,838,1186,1534(+overflow) |
| platform graphic img box | [148,7068,1232,532] |
| testimonial headshots 48x48 | x=740,2004(offscreen),... y=7895 |
| dev h2 (dark) | [148,8514,670,40] |
| what's-happening h2 | [148,10855,348,35] |
| squeezy carousel | [148,10989,1232,567]; canvas [148,10989,1232,460]; details row [148,11481,1232,75] |
| book of the week | [148,11818,1232,585] |
| ready CTA h2 span | [148,12595,479,35] |
| footer column heads | "Products and pricing" [148,12943,271,27], "Solutions" [460,12943], "Developers" [460,13562], "Integrations..." [772,12943], "Resources" [772,13106], "Company" [1084,12943], "Support" [1084,13130] — 4 columns of width 271-272, column pitch 312 |

Header internals: logo svg [149,26,60,25]; nav items Products
[237,18,77,40], Solutions [338,18,78,40], Developers [440,18,91,40],
Resources [555,18,86,40], Pricing [665,19,44,38]; Sign in
[1150,18,84,40] (color rgb(83,58,253), font 14px); Contact sales
button [1242,16,137,44] filled #635bff-family (see palette).

Hero CTA styles [cmd: eval computed styles]:
- Get started: bg rgb(83,58,253), color #fff, radius 4px, padding
  15.5px 24px 16.5px, font 16px/16px w400, h=48.
- Google: bg rgba(255,255,255,0.65), color rgb(83,58,253), border
  1px solid rgb(185,185,249), radius 4px.

## 2. Responsive reflow

Breakpoints in the CSS: **max-width:639px (mobile), 640-939
(tablet), min-width:940px (desktop)**; a few extras at 840, 1264,
1300. [cmd: grep '@media' over mirror css — measurement of build
output, general breakpoints only]

Measured document heights and per-section `[top,height]` on fresh
loads (clock paused ~660 ms) at test widths [cmd: open fresh window
per width, resize, eval section walk]:

| width | doc h | sections [top,h] in order 0..9 |
|---|---|---|
| 360 | 18653 | [0,66],[66,519],[585,3245],[3830,688],[4518,776],[5294,4805],[10099,3100],[13199,1917],[15116,720],[15836,2817] |
| 768 | 14373 | [0,66],[66,628],[694,1832],[2526,512],[3038,810],[3848,4063],[7910,2509],[10420,1844],[12263,523],[12786,1587] |
| 1024 | 13313 | [0,76],[76,636],[712,1795],[2507,560],[3067,860],[3927,3977],[7904,2293],[10197,1663],[11860,380],[12240,1073] |
| 1280 | 13890 | [0,76],[76,636],[712,2177],[2889,560],[3449,968],[4417,3973],[8390,2334],[10723,1733],[12457,380],[12837,1053] |
| 1528 | 13932 | (table in §1) |
| 1920 | 13932 | identical to 1528 (container is max-width capped; only outer gutters grow) |

Header is 66px tall below 940 (hamburger nav) and 76px at ≥940
(full nav). Hero: single-column at all widths; the wave art scales.
Bento grid: 1 column at 360, 2 at 768, 3 at ≥940 (first card spans
2). Footer: stacked columns at 360, 2-col at 768, 4-col at ≥940.

Caution: an earlier same-window resize sequence gave slightly
different heights at 768/1024 than fresh loads (lazy-loaded state
differs). The gate uses fresh loads per viewport; build to the
fresh-load numbers above.

## 3. Type scale (computed, at 1528)

Font stack everywhere: `sohne-var, "SF Pro Display", sans-serif`.
Body default 16px w300 color rgb(0,0,0). Design weights: normal=300,
bold=400 (tokens `--hds-font-weight-*`).

| role | size/line-height | weight | letter-spacing | color |
|---|---|---|---|---|
| h1 hero (`hds-heading--xl` at desktop) | 48px / 55.2px | 300 | -0.96px | see §7 dual-layer trick |
| h2 section (`--xxl` scale renders 32px/35.2px here) | 32px / 35.2px | 300 | -0.64px | rgb(6,27,49) |
| lead paragraph (subdued half of h2) | 32px / 35.2px | 300 | -0.64px | rgb(100,116,141) |
| h3 card | 26px / 29.12px | 300 | -0.26px | rgb(6,27,49) |
| nav link | 14px / 14px | 400 | 0 | rgb(6,27,49) |
| button label | 16px / 16px | 400 | 0 | #fff on solid |
| GDP eyebrow | 16px | 400 | 0 | rgb(0,0,0), `font-feature-settings:"ss01"`, class `tabular-nums--tight` |
| stat value (stats menu) | (menu) — values inherit large size; measure per diff | 300 | | animates white→rgb(6,27,49) etc. |
| dark-section gradient stats (500M+/10K+/150K+) | 48px | 300 | | gradient text, see §5 |
| footer heading | 16px / 19.2px | 400 | 0 | rgb(6,27,49) |
| footer link | 16px / 20px | 300 | 0 | rgb(80,97,122) |

Full HDS font tokens (rem, mobile-first defaults; desktop media
queries scale up) are in `recon/design-tokens.json`
(`--hds-font-heading-*`, `--hds-font-text-*`).

## 4. Palette / spacing / radii / shadows (design tokens)

701 custom properties captured in `recon/design-tokens.json`
[cmd: eval walk over document.styleSheets cssRules]. Key values:

Brand/action colors:
- brand-600 `#533afd` (primary button bg = rgb(83,58,253))
- brand hover 500 `#665efd`; brand-200 `#b9b9f9` (Google-btn border)
- link/action text `rgb(83,58,253)`
- heading ink `rgb(6,27,49)` (= neutral-990 `#061b31`)
- subdued text `rgb(100,116,141)` (neutral-500 `#64748d`)
- footer link `rgb(80,97,122)` (neutral-600 `#50617a`)
- neutral-25 `#f8fafd` (footer/CTA band bg = rgb(248,250,253))
- neutral-50 `#e5edf5` (container guide borders rgb(229,237,245))
- dark section bg `rgb(13,23,56)` ≈ `#0d1738` (neutralDark-990)
- accent gradient stops `#bdb4ff`, `#643afd`, `#533afd`

Spacing scale: `--hds-space-core-N` = N/100*8px grid: 25→2px, 50→4,
100→8, 150→12, 200→16, 300→24, 400→32, 500→40, 550→44 (button
height), 600→48, 700→56, 800→64, 1200→96 ... 2500→200.
Layout: content maxWidth 1264px, content margin 16px, gap 16px.

Radii: xs 2px, sm 4px (buttons), md 6px, lg 16px, xl 32px,
round 99999px.

Shadows (two-layer, top+bottom):
- md: `0 6px 22px <top>, 0 4px 8px <bottom>`
- lg: `0 15px 40px -2px, 0 5px 20px -2px`
- xl: `0 20px 80px -16px, 0 10px 60px -16px`
- canary UI shadow: `0px 16px 32px rgba(50,50,93,.12)`
(colors resolve via `--hds-color-surface-bg-shadow`; see tokens file.)

Stats-section time-of-day radial gradients (exact, measured)
[cmd: eval computed backgroundImage of each
`.stats-animation-gradient__gradient--*`; full strings in
`recon/stats-gradients.json`]:

- pre-dawn: `radial-gradient(103.24% 102.63% at 50% 102.63%, rgb(72,111,253) 0, rgb(127,129,243) 9.84%, rgb(196,137,255) 20.83%, rgb(218,192,255) 34.13%, rgb(234,220,255) 44.86%, rgb(249,246,255) 58.59%, rgb(248,250,253) 100%)`
- sunrise: `radial-gradient(102.68% 99.11% at 50% 104.6%, rgb(203,131,255) 0, rgb(255,144,185) 15.77%, rgb(255,201,119) 30.62%, rgb(255,215,155) 38.04%, rgb(255,241,220) 50.11%, #fff 63.1%, rgb(252,253,254) 77.95%, rgb(248,250,253) 98.81%)`
- daytime: `radial-gradient(102.84% 104.98% at 50% 104.98%, rgb(0,113,193) 1.33%, rgb(96,168,226) 15.71%, rgb(180,216,255) 33.15%, rgb(217,235,255) 45%, rgb(248,250,253) 60%)`
- dusk: `radial-gradient(102.83% 103.24% at 49.98% 104.51%, rgb(255,180,81) 0, rgb(239,198,128) 16.73%, rgb(180,216,255) 33.03%, rgb(210,232,255) 43.38%, rgb(250,253,255) 59.16%, rgb(253,254,255) 76.24%, rgb(248,250,253) 100%)`
- sunset: `radial-gradient(103.12% 100% at 50% 100%, rgb(255,165,119) 0, rgb(255,144,161) 15.52%, rgb(221,173,255) 30.09%, rgb(236,216,255) 45.72%, rgb(245,234,255) 54.96%, rgb(248,250,253) 88.16%)`
- night: `radial-gradient(102.82% 106.44% at 50% 106.44%, rgb(252,253,254) 1.11%, rgb(103,99,228) 28.73%, rgb(69,59,179) 45.76%, rgb(41,34,125) 63.37%, rgb(30,32,100) 78.67%, rgb(20,30,75) 100%)`

All six gradient divs exist stacked; only the active one has
opacity 1 (crossfade on change).

Dark-section gradient stat text (background-clip:text):
- orange 500M+: `linear-gradient(68deg, rgba(83,58,253,0.08) 0.78%, rgba(255,140,108,0.8) 30.61%, rgba(218,75,254,0.8) 79.02%)`
- pink 10K+: `linear-gradient(73.3deg, rgba(218,75,254,0.8) 9.85%, rgba(113,92,255,0.48) 61.94%)`
- purple 150K+: `linear-gradient(74.71deg, rgba(83,58,253,0.08) -215.1%, rgba(255,140,108,0.8) -169.26%, rgba(218,75,254,0.8) -12.8%, rgba(113,92,255,0.8) 18.59%, rgba(83,58,253,0.8) 39.04%)`

## 5. Content

Complete visible text content per section, with per-node tag/class/
rect/font-size/weight/color, is in `recon/text-content.json`
(324 text nodes) [cmd: eval TreeWalker dump]. Headings summary:

- h1: "Financial infrastructure to grow your revenue." +
  "Accept payments, offer financial services, and implement custom
  revenue models—from your first transaction to your billionth."
- h2 x5: Flexible solutions for every business model. / The backbone
  of global commerce / Powering businesses of all sizes. / Reliable,
  extensible infrastructure for every stack. / What's happening
- h3 x24: see text-content.json (bento cards, enterprise stories
  Hertz/URBN/Instacart/LeMonde, startups, SaaS, dev pillars, squeezy
  slides, book of the week).

135 visible inline SVGs (logos, icons, charms); full outerHTML for
every one is in `recon/svg-inventory.json` (224 KB) — builders copy
these verbatim (they are part of the DOM, not fetched assets), which
is measurement-derived markup, allowed. Marquee logo set (18 logos
x2 for the wrap copy): OpenAI, Amazon, Nvidia, Ford, Coinbase,
Google, Shopify, Mindbody, MetLife, Ramp, Marriott, Figma,
WooCommerce, Vercel, Uber, Anthropic, Lightspeed, Cursor. Each
`<li>` is exactly 172px wide, 36 li total, UL scrollWidth 6192px.

## 6. Asset inventory

Raster: 41 unique basenames, 221 files on the mirror disk (multiple
@w= variants). **None of them load in the mirror** (404 URLs, §0),
so for mirror-parity the clone needs zero raster bytes. If the
target is ever re-pointed at a fixed mirror where images load, the
legally-safe sources are: Stripe's public CDN URLs at
images.stripeassets.com (same content, hotlink) or re-download via
`fetch-reference.sh`. Pixel-exactness then matters for:
wave-fallback-desktop.png (hero), DatavizStatic3x.png (stats globe
fallback), the 4 enterprise-accordion-*.png, sessions-2026 banners,
payment/Connect bento backgrounds, 8 startup-card logos, 4
testimonial headshots, 8 the-happenings thumbs, WorkInProgressIcon,
book cover. `<img>` layout boxes (CSS px) are in
`recon/img-assets.json` [cmd: eval document.images walk].

Fonts: `sohne.woff2` (already in `scripts/demo/clone/`) — must be
byte-identical for glyph rasterization parity (it is the single
loaded font, §0). SourceCodePro: do NOT ship (never loads, unused).

Favicons: not needed for viewport pixels.

## 7. Notable rendering constructs (build these exactly)

1. **Dual-layer hero h1.** Two identical h1 elements stacked at
   [252,234,958,220]:
   - background layer `hero-section__title--background`: color
     rgb(129,184,26) (green), plain.
   - foreground layer `hero-section__title--foreground`: color
     rgba(0,14,255,0.5), `mix-blend-mode: hard-light`, z-index 2.
   The `<em>` first sentence in each is rgb(6,27,49) solid.
   Green+blue hard-light composite produces the final blue-violet
   gradient look over the wave art. Copy exactly; do not substitute
   a background-clip gradient.
2. **GDP eyebrow ticker** (`tabular-nums--tight`, ss01): value =
   `0.016 + 6.464502655389533e-11 * tSec + 1e-10 * simplexNoise1D(tSec)`
   percent, where tSec = (Date.now() - epoch("2026-02-20"))/1000,
   formatted to exactly 8 fraction digits, suffix `%`. Updated by
   `setInterval(H, 1250)`. Digit changes animate as per-character
   outgoing/incoming span pairs translating vertically 325ms
   cubic-bezier(0.33,1,0.68,1) with 50ms stagger per digit position
   (that is the family of ~22 span transform animations in the
   declared table). The noise permutation table is seeded with
   `Math.random()` at module init (hazard H-1). A hidden HTML
   comment describing the calculation is embedded via a
   `script type="text/comment"` node.
3. **Logo marquee** (`ul.logo-carousel__marquee`): rAF integrator
   sets `style.transform = translateX(F)` where
   `F -= 0.03 * dtMs` per frame (0.03 px/ms = 30 px/s) and wraps by
   `while (F < -W) F += W; while (F > 0) F -= W;` with W = half the
   scroll width = 3096px. Decompiled loop [cmd: read chunk 75351]:
   momentum after drag decays by `T *= N ** (dt/16.667)`, and
   `velocity 0.03` is used only when not hovered/dragged and in
   view (IntersectionObserver-gated start). Observed:
   velocity -29.99..-30.00 px/s, period 103.2 s, r²=1.0
   [cmd: `hwatu motion --observe --ms 2500` and `--ms 12000`,
   wrap-hunt reported period_s 103.2]. Nominal period = 3096/30 =
   103.2 s exactly. Reduced-motion sets speed 0.
4. **Stats section time-of-day**: variant chosen at hydration by
   local hour: 5-8 pre-dawn, 8-11 sunrise, 11-16 daytime, 16-20
   dusk, 20-23 sunset, else night [decompiled from index chunk:
   `let e=new Date().getHours(); e>=5&&e<8?"pre-dawn":...`]. Class
   `stats-section--time-<v>` + active gradient div. `night` also
   flips the section to dark mode (`stats-section--dark`). The SSR
   HTML carries `--time-night`; hydration replaces it with the
   client-local variant (hazard H-2). WebGL globe (`DataViz`) is
   loaded ssr:false; in this environment `supportsWebGL` path still
   yields a hidden globe container (`display:none`) and the static
   fallback `<img DatavizStatic3x>` (which 404s → paints nothing).
   The visible remains: gradient + stat menu + title.
5. **Enterprise accordion (customer stories)**: 4 rows
   Hertz/URBN/Instacart/LeMonde. Open row's content div animates
   `maxHeight` 500ms cubic-bezier(0.65,0,0.35,1); closed ones get
   `visibility` steps(1) after 500ms delay. Story pill buttons
   animate width 40px↔(text+50)px 400ms cubic-bezier(0.3,0,0.2,1)
   with bg/border-color fades and text/icon opacity 400ms.
   First-paint state: Hertz open (`customer-story-button--open`,
   width 166px), others 40px.
6. **Squeezy carousel** (What's happening): canvas-drawn slide
   deck (2D context) + absolutely-stacked `__item-details` rows
   (active opacity 1, inactive 0, inactive ones also
   translateX(-1232px)). Canvas stays default 300x150 backing size,
   CSS-stretched to 1232x460, and paints nothing in this env.
   Details text for slide 0: "Businesses on Stripe generated $1.9T
   in 2025." etc.
7. **Book of the week**: image-placeholder and content divs fade in
   opacity 500ms linear once loaded/visible.
8. **Bento card hover borders** (`__border-color` with matrix
   translate + opacity 0) are hover/pointer-driven; at first paint
   opacity 0 — irrelevant to static diffs but present in DOM.
9. **`lazy-animation` divs** (opacity 0 at load, 12+ instances,
   tops 973..10250): IntersectionObserver-triggered reveals.
   Trigger: element enters viewport (threshold from `useIO`
   helper, default 0, rootMargin 0%). These affect pixels at any
   scroll position below the fold, so the clone must reproduce both
   the pre-reveal (opacity 0) and post-reveal states with the same
   IO semantics. Under `hwatu clock step` the daemon pumps IO with
   virtual timestamps, so reveal timing is deterministic given the
   same scroll choreography.
10. **detect-scroll CSSAnimation** on `nav#hds-navigation-menu`
    (`animation-name: detect-scroll`, duration auto/scroll-timeline):
    used as a scroll-position detector for nav styling.
11. **Third-party iframes**: two 4x4px `iframe.ThirdPartyFrame`
    appended at [0,13932] (PrivacyCompliance, GoogleTagManager,
    b.stripecdn.com URLs). They occupy 4x4 boxes at the very bottom
    (document height already includes them). Clone: two 4x4 iframes
    (about:blank or local stub) at the same place.

## 8. Declared animation table

Raw dump: `recon/motion-declared.json` [cmd: `hwatu motion --id <ref>`
on a fresh load, clock paused at ~660ms]. 70 entries at first paint;
they dedupe to ~30 unique (target,property) pairs — the GDP ticker
contributes 22 span pairs that grow to ~199 as ticks accumulate, so
"32 declared" from the plan is a settled-page dedupe count; gate on
the deduped inventory, not the raw entry count.

| target | property | dur ms | delay | easing | fill | notes |
|---|---|---|---|---|---|---|
| span.stats-section__active-indicator--top/--bottom (x2) | background-image (transparent→rgb(67,4,234) center sweep) | 400 | 0 | cubic-bezier(0.4,0,0.2,1) | backwards | stat-menu active underline |
| h2.stats-section__title | color #fff→rgb(6,27,49) | 1200 | 0 | cubic-bezier(0.65,0,0.35,1) | backwards | on variant change |
| p#stat-{payment-methods,payments-volume,historical-uptime,active-subscriptions}-value + span#...-description (8) | color | 300 | 0 | cubic-bezier(0.25,1,0.5,1) | backwards | two color pairs: #fff→rgb(6,27,49) and rgb(100,128,178)→rgb(125,139,164) |
| a.stats-menu__stat-description__link | background-color | 500 | 0 | cubic-bezier(0.25,1,0.5,1) | backwards | |
| a.customer-story-button--open | width 40↔166px | 400 | 0 | cubic-bezier(0.3,0,0.2,1) | backwards | |
| div.customer-story-button__container (x4) | width | 400 | 0 | cubic-bezier(0.3,0,0.2,1) | backwards | |
| div#detail-customer-content-{Hertz,URBN,Instacart,LeMonde} | maxHeight (open) / visibility steps(1) delay 500 (close) | 500/0 | 0/500 | cubic-bezier(0.65,0,0.35,1) / steps(1) | backwards | |
| a.customer-story-button (x3 closed) | background-color + 4 border colors + width | 400 | 0 | cubic-bezier(0.3,0,0.2,1) | backwards | |
| div.customer-story-button__text/__icon (x3 pairs) | opacity | 400 | 0 | cubic-bezier(0.3,0,0.2,1) | backwards | |
| nav#navigation-menu | detect-scroll keyframe | auto | 0 | linear | none | CSSAnimation, scroll detector |
| div.book-of-the-week__picture placeholder | opacity | 500 | 0 | linear | none | |
| div.book-of-the-week__content | opacity | 500 | 0 | linear | none | |
| GDP digit spans (11 outgoing + 11 incoming at first paint) | transform translateY | 325 | 0,50,...,500 (50ms stagger) | cubic-bezier(0.33,1,0.68,1) | both | re-created every 1250ms tick |

## 9. Observed (script-driven) motion table

[cmd: `hwatu motion --id <ref> --observe --ms 2500` and `--ms 12000`;
meta: 300/600 frames under virtual time, wrap_hunt engaged]

| target | model | axis | velocity | period | r² | verdict |
|---|---|---|---|---|---|---|
| ul.logo-carousel__marquee | linear+wrap | x | **-29.99..-30.00 px/s** | **103.2 s** (wrap 3096px) | **1.0** | the only real script animation; matches source constant 0.03 px/ms exactly |
| span (GDP digits) | linear (junk fit) | y | -0.03..-0.97 | — | 0.03-0.15 | transient tick animation caught mid-flight; not a continuous motion; model per §7.2 |
| div#customer-URBN + siblings layout | linear (junk fit) | y | 0.2-3.2 | — | 0.1-0.4 | layout ripple from accordion maxHeight settling right after load; decays to ~0 in later windows |
| div.customer-stories__customer-summary-action | linear (junk) | x | -0.3..-4.4 | — | 0.06-0.23 | same ripple |

Cross-check vs source: marquee integrator `F -= 0.03*dt(ms)`, wrap
width = ul.scrollWidth/2 = 6192/2 = 3096px → 30 px/s, period
3096/30 = 103.2 s. Observed matches within 0.03%. The plan's
"~29.99 px/s, wrap 3096, period ~104s" is confirmed (104→103.2 with
the better fit).

No other element moves autonomously: 12 s observation windows find
nothing else with r² > 0.5. Squeezy carousel and events carousel do
NOT auto-advance (no autoplay interval found in source; none
observed).

## 10. Hazard census and mitigations

| id | source | where | visually relevant? | mitigation |
|---|---|---|---|---|
| H-1 | `Math.random()` — noise permutation table `Array.from({length:256},()=>Math.floor(256*Math.random()))` | GDP eyebrow ticker noise term (index chunk, module init) | **YES but bounded**: noise contributes `1e-10 * f(t)` percent where f∈(-1,1) → at most ±1 unit in the 8th fraction digit; digits 1-7 are load-independent. Two loads can differ in the last rendered digit. | (a) Preferred: seedable `Math.random` in the hwatu clock shim (toolsmith item) so ref and clone tables match; (b) fallback: prover masks the final digit cell of the eyebrow (~9x21px), documented. The clone must implement the same formula + a same-seed noise table. |
| H-2 | `new Date().getHours()` | stats-section time-of-day variant (6 variants, §7.4) | **YES, large** (whole section gradient + optionally dark mode at night) | `Date.now` IS behind the hwatu clock shim: `clock set 0` pins wall-clock at daemon-launch epoch. Protocol: launch/verify at a fixed virtual wall time; clone reads the same shimmed Date and computes the same variant. Both windows on the same daemon+set see identical hours. Never gate across a 5/8/11/16/20/23 local-hour boundary without `clock set`. NOTE the SSR class is `--time-night`; hydration replaces it, so first-paint-before-JS differs from post-hydration. Diff only after hydration settles. |
| H-3 | `Date.now()` continuous | GDP ticker value (grows ~6.5e-11 %/s) | YES (digits 8 fraction places) | Same clock shim; at equal virtual times the deterministic term is identical. Residual risk is only H-1's noise digit. |
| H-4 | hero WebGL canvas (`hero-wave-animation__canvas`, 1393x712 CSS) | hero | **NO in this environment**: readback fully transparent (0/180000 px alpha>0); the 404 fallback img also paints nothing | Clone ships an equally-empty canvas-sized block (or a canvas never drawn to). Verify with hero-region diff at t=0. If a future WebKit paints it, escalate to toolsmith (GPU nondeterminism → mask). |
| H-5 | squeezy 2D canvas (300x150 backing, 1232x460 CSS) | what's happening | **NO**: all-zero pixels; slide text lives in DOM details rows | Same as H-4: size-matched empty canvas. |
| H-6 | Math.random in dot-globe/data-viz workers (`dotGeneration.worker.js` 404s; 6e3/1e4 random points, random arc palettes, `getRandomUIData`) | stats globe, wave internals | **NO**: worker file 404s in mirror; globe container display:none; fallback img 404s | Nothing to do for mirror parity. Documented for future live-parity work. |
| H-7 | Math.random in framework chunks (three.js LineSegments default material color, React internals) | none visible | NO (default material colors are always overridden; framework uses random for keys/ids not pixels) | none needed |
| H-8 | network time / remote fetches: q.stripe.com analytics beacons, images.stripeassets.com (wave/palette/map_dots), b.stripecdn.com css/js chunks (50582, 90968, 98932, 36322 load remotely!) | page behavior depends on internet availability | **YES for load determinism**: 4 JS chunks + 2 css come from live CDN. If offline or CDN changes, behavior may drift | Mirror gap. Mitigation: run the gate with these requests either (a) consistently reachable (they are pinned by content-hash filenames, so drift risk is low), or (b) blocked for BOTH windows (hwatu adblock/hosts) after confirming blocking doesn't change pixels. Prover should pick one and record it. Recommend extending the mirror to include these 6 files (fetch-reference.sh fix) — flagged for toolsmith/harness. |
| H-9 | ThirdPartyFrame iframes (remote HTML) | 4x4px at page bottom | marginal (4x4 px, below fold at 13932) | clone uses same-size local iframes; if remote loads change their pixels, mask the 2 tiny rects or block the hosts on both sides. |
| H-10 | cross-load hydration timing (accordion maxHeight ripple ~first 2s) | layout ±3px early | YES if diffed too early | Protocol: wait-load, then let hydration settle (expect stable doc height), then `clock set`, then photograph. Harness owns this sequencing. |
| H-11 | cookie/localStorage state (`[stripe-cookies]` warnings) | none seen (banner suppressed on localhost) | NO on this mirror | none; keep localhost origin so the cid-cookie refusal path stays identical. |

**Explicit Math.random verdict (task item 6):** Math.random IS used
in visually-relevant code on the live site (globe arcs, dot cloud,
wave), but in the frozen mirror the only *visible* consumer is the
GDP ticker's noise table (H-1), whose visual effect is confined to
the 8th fraction digit (±1 unit) of the eyebrow figure. Everything
else that consumes Math.random either fails to load (worker 404),
is display:none (globe), or paints nothing (WebGL canvases).

## 11. First-paint interaction states

- Enterprise accordion: Hertz open, its pill 166px wide, others
  40px; detail images maxHeight-collapsed to 0 except active.
- `lazy-animation` reveals (12+): opacity 0 until IO fires;
  everything above y≈828 has already revealed by settle time.
  Below-fold screenshots therefore depend on the scroll protocol:
  scrolling to y triggers reveals for elements entering view. The
  harness matrix must use the same scroll sequence on both windows.
- Squeezy details: slide 0 visible, slides 1..7 opacity 0 +
  translateX(-1232px).
- Stats menu: first stat active (white text on gradient, active
  indicator sweep played once).
- Nav: transparent header; `detect-scroll` animation drives a
  sticky/solid state when scrolled (verify with scrolled diffs).
- Hover-only artifacts (bento border-color, story-button hover,
  logo-carousel drag cursor) never affect headless first paint.

## 12. Builder checklist (order of attack)

1. Static skeleton: 10 sections at the §1 rects, container guides,
   fonts, palette. Verify per-section with `diff --heatmap`.
2. Text content from `recon/text-content.json`; SVGs verbatim from
   `recon/svg-inventory.json`.
3. Broken-image placeholders at `recon/img-assets.json` rects.
4. Dual-layer h1 (§7.1) and dark-section gradient text (§4).
5. Stats gradients + time-of-day switch on shimmed hour (§7.4).
6. Declared animations from §8 (CSS transitions/WAAPI with the
   exact beziers).
7. Marquee integrator (§7.3): 0.03 px/ms, wrap 3096, IO-gated,
   hover/drag handlers optional for pixel parity but reduced-motion
   guard required.
8. GDP ticker (§7.2) with seedable noise (coordinate with
   toolsmith on H-1).
9. IO-gated `lazy-animation` reveals with the same thresholds.
10. Two 4x4 bottom iframes.

---
Raw artifacts (same directory tree, committed):
`recon/design-tokens.json` (701 tokens), `recon/text-content.json`,
`recon/svg-inventory.json` (135 visible SVGs, outerHTML),
`recon/img-assets.json`, `recon/motion-declared.json`,
`recon/motion-observed.json`, `recon/stats-gradients.json`.
