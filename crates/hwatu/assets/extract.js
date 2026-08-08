// extract.js — runs inside the target page via `hwatu eval`.
// Async: sweeps the page (priming lazy content + scroll reveals and
// harvesting canvas frames while they are painted), settles animations
// (entrance reveals to their END state, infinite loops paused at 0 —
// pausing everything at 0 freezes reveals at opacity:0 and produces a
// faithful copy of a broken page), then serializes the rendered DOM.
return (async () => {
const canvasFrames = new Map(); // canvas element -> dataURL
{
  // Tag canvases up front so frames can be matched later.
  document.querySelectorAll('canvas').forEach((c, i) => { c.dataset.hwatuCanvas = i; });
  const harvest = () => {
    for (const c of document.querySelectorAll('canvas')) {
      const r = c.getBoundingClientRect();
      const visible = r.bottom > 0 && r.top < innerHeight && r.width > 0;
      if (!visible) continue;
      try {
        const data = c.toDataURL('image/png');
        // Keep the largest capture (later frames of a lazy canvas
        // usually have more drawn on them than the first).
        const prev = canvasFrames.get(c);
        if (!prev || data.length > prev.length) canvasFrames.set(c, data);
      } catch (e) { /* tainted */ }
    }
  };
  const H = document.documentElement.scrollHeight;
  for (let y = 0; y <= H; y += 400) {
    scrollTo(0, y);
    await new Promise(r => setTimeout(r, 60));
    harvest();
  }
  scrollTo(0, 0);
  await new Promise(r => setTimeout(r, 800));
  harvest();
}

// Scroll-coupled text highlighting is not safe to replay in the static
// clone yet: sites often implement it with passive scroll listeners that
// schedule requestAnimationFrame style writes. Detect the dependency and
// record entry/midpoint/exit evidence instead. This intentionally focuses
// on multi-token text style changes in normal content, so a fixed/sticky
// navbar that merely hides or shows on scroll is not reported as a quote
// highlight.
async function detectScrollEffects() {
  const maxScroll = Math.max(0, document.documentElement.scrollHeight - innerHeight);
  if (maxScroll < 80) return [];
  const waitScroll = async (y) => {
    scrollTo(0, Math.max(0, Math.min(maxScroll, y)));
    const ev = new Event('scroll');
    window.dispatchEvent(ev);
    document.dispatchEvent(new Event('scroll'));
    // Scroll handlers commonly schedule style writes in rAF. Give one
    // frame a bounded chance to run; the timeout keeps headless capture
    // from hanging if rAF is throttled.
    await Promise.race([
      new Promise(r => requestAnimationFrame(() => r('frame'))),
      new Promise(r => setTimeout(() => r('timeout'), 120)),
    ]);
    await new Promise(r => setTimeout(r, 40));
  };
  const ownText = (el) => [...el.childNodes]
    .filter(n => n.nodeType === Node.TEXT_NODE)
    .map(n => n.textContent.trim()).join(' ').replace(/\s+/g, ' ').trim();
  const navish = (el) => {
    for (let n = el; n && n !== document.body; n = n.parentElement) {
      const tag = n.tagName && n.tagName.toLowerCase();
      if (tag === 'nav' || tag === 'header' || n.getAttribute('role') === 'navigation') return true;
    }
    return false;
  };
  const positionedChrome = (el) => {
    for (let n = el; n && n !== document.body; n = n.parentElement) {
      const st = getComputedStyle(n);
      if (st.position === 'fixed' || st.position === 'sticky') return true;
    }
    return false;
  };
  const all = [...document.querySelectorAll('body *')].slice(0, 12000);
  const candidates = [];
  const visualCandidate = (el, st) => {
    const r = el.getBoundingClientRect();
    const area = Math.max(0, r.width) * Math.max(0, r.height);
    if (area < Math.min(innerWidth * innerHeight * 0.015, 12000)) return false;
    if (st.display === 'inline' || st.position === 'fixed') return false;
    return true;
  };
  for (const el of all) {
    if (navish(el)) continue;
    const text = ownText(el);
    const st = getComputedStyle(el);
    if (st.display === 'none' || st.visibility === 'hidden') continue;
    const isText = text.length >= 1 && text.length <= 80;
    const isVisual = !isText && visualCandidate(el, st);
    if (isText && positionedChrome(el)) continue;
    if (!isText && !isVisual) continue;
    candidates.push({ el, text, candidateKind: isText ? 'text' : 'visual' });
    if (candidates.length >= 5000) break;
  }
  if (candidates.length < 1) return [];
  const styleOf = (el) => {
    const st = getComputedStyle(el);
    const r = el.getBoundingClientRect();
    return {
      color: st.color,
      backgroundColor: st.backgroundColor,
      opacity: st.opacity,
      textShadow: st.textShadow,
      transform: st.transform,
      clipPath: st.clipPath,
      borderRadius: st.borderRadius,
      rect: { top: Math.round(r.top), left: Math.round(r.left), width: Math.round(r.width), height: Math.round(r.height) },
    };
  };
  const styleKey = (s) => `${s.color}|${s.backgroundColor}|${s.opacity}|${s.textShadow}|${s.transform}|${s.clipPath}|${s.borderRadius}`;
  const coarse = [...new Set([0, .125, .25, .375, .5, .625, .75, .875, 1]
    .map(f => Math.round(maxScroll * f)))];
  const seen = candidates.map(() => []);
  for (const y of coarse) {
    await waitScroll(y);
    for (let i = 0; i < candidates.length; i++) {
      const { el } = candidates[i];
      if (!el.isConnected) continue;
      const style = styleOf(el);
      seen[i].push({ y, style, key: styleKey(style) });
    }
  }
  const changed = [];
  for (let i = 0; i < seen.length; i++) {
    const variants = new Set(seen[i].map(s => s.key));
    if (variants.size >= 2) changed.push({ i, ...candidates[i], observations: seen[i] });
  }
  const effects = [];
  let effectIndex = 0;
  const cssPath = (el) => {
    if (el.id) return `#${CSS.escape(el.id)}`;
    if (el.dataset.hwatuScrollEffect) return `[data-hwatu-scroll-effect="${el.dataset.hwatuScrollEffect}"]`;
    const parts = [];
    for (let n = el; n && n !== document.body && parts.length < 5; n = n.parentElement) {
      let part = n.tagName.toLowerCase();
      const cls = [...n.classList].filter(Boolean).slice(0, 2);
      if (cls.length) part += '.' + cls.map(CSS.escape).join('.');
      const parent = n.parentElement;
      if (parent) part += `:nth-child(${[...parent.children].indexOf(n) + 1})`;
      parts.unshift(part);
    }
    return 'body > ' + parts.join(' > ');
  };
  const clamp = (y) => Math.max(0, Math.min(maxScroll, Math.round(y)));
  // Pin candidates: a sizeable element whose viewport top plateaus while scrollY advances.
  const pinRoots = [];
  for (let ci = 0; ci < candidates.length; ci++) {
    const c = candidates[ci];
    if (pinRoots.some(root => root.contains(c.el) || c.el.contains(root))) continue;
    if (c.candidateKind !== 'visual' && !c.el.children.length) continue;
    const obs = seen[ci].filter(o => o.style.rect && o.style.rect.height > 40);
    let best = null;
    for (let a = 0; a < obs.length; a++) for (let b = a + 2; b < obs.length; b++) {
      const slice = obs.slice(a, b + 1);
      const dy = slice[slice.length - 1].y - slice[0].y;
      const tops = slice.map(o => o.style.rect.top);
      const span = Math.max(...tops) - Math.min(...tops);
      if (dy >= innerHeight * 0.45 && span <= 3 && tops[0] > -innerHeight * 0.2 && tops[0] < innerHeight * 0.85) {
        if (!best || dy > best.endY - best.startY) best = { startY: slice[0].y, endY: slice[slice.length - 1].y, top: Math.round(tops.reduce((x, y) => x + y, 0) / tops.length) };
      }
    }
    if (!best) continue;
    const root = c.el.parentElement || c.el;
    root.dataset.hwatuScrollEffect = effectIndex;
    effects.push({
      kind: 'scroll-pin', selector: `[data-hwatu-scroll-effect="${effectIndex}"]`, pinnedSelector: cssPath(c.el),
      dependency: 'viewport rect plateau across scroll sweep; replay uses stable ancestor rect and child translate, preserving report-only fallback if selector misses',
      startY: best.startY, endY: best.endY, desiredViewportTop: best.top,
    });
    pinRoots.push(root);
    effectIndex += 1;
    if (effects.length >= 6) break;
  }
  // Non-text transform/clip/opacity scrubs: store raw endpoints for direct replay.
  for (const c of changed.filter(x => x.candidateKind === 'visual').slice(0, 12)) {
    const obs = c.observations;
    const keys = obs.map(o => o.key);
    const firstChange = keys.findIndex(k => k !== keys[0]);
    if (firstChange < 1) continue;
    const lastChange = keys.length - 1 - [...keys].reverse().findIndex(k => k !== keys[keys.length - 1]);
    const from = obs[Math.max(0, firstChange - 1)];
    const to = obs[lastChange];
    c.el.dataset.hwatuScrollEffect = effectIndex;
    effects.push({
      kind: 'scroll-coupled-visual-style', selector: `[data-hwatu-scroll-effect="${effectIndex}"]`,
      dependency: 'non-text paint-area element changes transform/clip-path/opacity after document scroll',
      progress: { startY: from.y, endY: to.y }, from: from.style, to: to.style,
    });
    effectIndex += 1;
  }
  // Time-based threshold tracks: constant-scroll dwell changes indicate wall-clock animation.
  const timeCandidates = candidates
    .map((c, ci) => ({ c, ci }))
    .filter(({ c, ci }) => c.candidateKind === 'text' || (seen[ci] || []).some(o => o.key !== ((seen[ci] || [])[0] && (seen[ci] || [])[0].key)))
    .slice(0, 80);
  await waitScroll(0);
  await new Promise(r => setTimeout(r, 180));
  for (const { c, ci } of timeCandidates) {
    const obs = seen[ci] || [];
    const coarseChange = obs.find(o => o.key !== (obs[0] && obs[0].key));
    const r0 = c.el.getBoundingClientRect();
    const docTop0 = r0.top + scrollY;
    const probeY = coarseChange ? coarseChange.y : clamp(docTop0 - innerHeight * 0.7);
    scrollTo(0, Math.max(0, Math.min(maxScroll, probeY)));
    window.dispatchEvent(new Event('scroll'));
    document.dispatchEvent(new Event('scroll'));
    const before = styleOf(c.el); const beforeKey = styleKey(before);
    await new Promise(r => setTimeout(r, 520));
    const after = styleOf(c.el); const afterKey = styleKey(after);
    if (beforeKey === afterKey) continue;
    c.el.dataset.hwatuScrollEffect = effectIndex;
    effects.push({
      kind: 'scroll-triggered-time-style', selector: `[data-hwatu-scroll-effect="${effectIndex}"]`,
      dependency: 'computed style changed during dwell at constant scrollY; replay uses threshold-triggered state machine',
      triggerY: Math.round(probeY), triggerLine: Math.round((before.rect.top / innerHeight) * 1000) / 1000,
      before, after,
    });
    effectIndex += 1;
    if (effects.filter(e => e.kind === 'scroll-triggered-time-style').length >= 12) break;
  }
  if (changed.filter(c => c.candidateKind === 'text').length < 3) {
    await waitScroll(0);
    return effects;
  }
  const groupRoot = (el) => {
    let best = el.parentElement || el;
    for (let n = best; n && n !== document.body; n = n.parentElement) {
      const text = (n.textContent || '').trim().replace(/\s+/g, ' ');
      const descendants = n.querySelectorAll('*').length;
      if (text.length >= 30 && text.length <= 800 && descendants <= 180 && !navish(n)) best = n;
    }
    return best;
  };
  const groups = new Map();
  for (const item of changed) {
    const root = groupRoot(item.el);
    const prev = groups.get(root) || [];
    prev.push(item);
    groups.set(root, prev);
  }
  for (const [root, items] of [...groups.entries()].sort((a, b) => b[1].length - a[1].length)) {
    if (!items.some(i => i.candidateKind === 'text')) continue;
    const uniqueText = new Set(items.map(i => i.text.toLowerCase()));
    if (items.length < 3 || uniqueText.size < 3 || navish(root)) continue;
    root.dataset.hwatuScrollEffect = effectIndex;
    const r = root.getBoundingClientRect();
    const docTop = r.top + scrollY;
    const docBottom = r.bottom + scrollY;
    const offsets = [
      { label: 'entry', y: clamp(docTop - innerHeight * 0.82) },
      { label: 'midpoint', y: clamp((docTop + docBottom) / 2 - innerHeight / 2) },
      { label: 'exit', y: clamp(docBottom - innerHeight * 0.18) },
    ];
    const samples = [];
    for (const off of offsets) {
      await waitScroll(off.y);
      samples.push({
        label: off.label,
        scrollY: Math.round(scrollY),
        elements: items.slice(0, 16).filter(i => i.el.isConnected).map(i => ({
          text: i.text,
          style: styleOf(i.el),
        })),
      });
    }
    // Replay fitting data: a scroll-coupled highlight is a pure function
    // of scrollY (stateless, deterministic). Tag each changed word,
    // sweep the activation window finely, and record per-word style
    // states so the materializer can fit
    //   progress = clamp01((innerHeight*A - rect.top) / (rect.height*B))
    // and emit a generated replay runtime.
    const wordEls = items.slice(0, 60).map(i => i.el).filter(el => el.isConnected);
    wordEls.sort((a, b) =>
      (a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING) ? -1 : 1);
    wordEls.forEach((el, k) => { el.dataset.hwatuScrollWord = k; });
    const words = wordEls.map((el, k) => ({ i: k, text: ownText(el), states: [] }));
    const fineFrom = offsets[0].y;
    const fineTo = offsets[2].y;
    if (fineTo - fineFrom >= 40 && wordEls.length >= 3) {
      const steps = 12;
      for (let s = 0; s <= steps; s++) {
        await waitScroll(fineFrom + (fineTo - fineFrom) * s / steps);
        const y = Math.round(scrollY);
        for (let k = 0; k < wordEls.length; k++) {
          words[k].states.push({ y, ...styleOf(wordEls[k]) });
        }
      }
    }
    // Coherent baseline: record each word's sampled ENTRY state; it is
    // baked into inline style after the final scroll-return below (the
    // page's own scroll handler would overwrite anything written now).
    // Without the bake the sweep's return-to-zero can serialize a
    // mid-reset frame (opacity-0 words) that no live scroll position
    // ever shows.
    effects.push({
      kind: 'scroll-coupled-text-style',
      selector: `[data-hwatu-scroll-effect="${effectIndex}"]`,
      dependency: 'text descendant computed style changes after document scroll; static clone records evidence but does not replay the scroll listener',
      changedTextNodes: items.length,
      sampleOffsets: offsets,
      textPreview: (root.textContent || '').trim().replace(/\s+/g, ' ').slice(0, 220),
      samples,
      geometry: { docTop: Math.round(docTop), height: Math.round(r.height), innerHeight },
      words,
    });
    effectIndex += 1;
    if (effects.length >= 3) break;
  }
  await waitScroll(0);
  return effects;
}
const scrollEffects = await detectScrollEffects();
// Bake the coherent baseline now that scrolling is over: each tagged
// word gets its sampled entry-state styles inline, so the serialized
// DOM is a still of a real scroll position (the effect's entry), never
// a mid-sweep reset. The replay runtime (or a reader) owns these
// subtrees from here; the pin pass below skips them.
for (const effect of scrollEffects) {
  const root = document.querySelector(effect.selector);
  if (!root) continue;
  for (const word of effect.words || []) {
    const entry = word.states && word.states[0];
    if (!entry) continue;
    const el = root.querySelector(`[data-hwatu-scroll-word="${word.i}"]`);
    if (!el) continue;
    el.style.color = entry.color;
    el.style.opacity = entry.opacity;
    el.style.backgroundColor = entry.backgroundColor;
    el.style.transform = entry.transform;
  }
}

// Freeze animation state for a still capture. Just pause, at the
// current frame: the sweep above already ran entrance reveals to
// completion naturally, and force-finish()ing is wrong for exclusive
// states (rotating headlines render every variant at once).
for (const a of document.getAnimations()) {
  try { a.pause(); } catch (e) { /* infinite timeline etc. */ }
}
await new Promise(r => setTimeout(r, 200));

// Returns { html, css, assets, canvases, base } as JSON.
// Serializes the *rendered* DOM so JS-built content survives without
// re-running the site's scripts locally.

// 1. Collect every rule the page actually loaded, via CSSOM.
//    Cross-origin sheets throw on .cssRules; record their href so the
//    materializer can fetch them server-side instead.
const sheets = [];
for (const sheet of document.styleSheets) {
  try {
    const rules = [...sheet.cssRules].map(r => r.cssText).join('\n');
    sheets.push({ base: sheet.href || location.href, text: rules });
  } catch (e) {
    if (sheet.href) sheets.push({ href: sheet.href });
  }
}

// 2. Freeze canvases and lazy media into data URLs / concrete attrs.
const canvases = [];
document.querySelectorAll('canvas').forEach((c) => {
  const i = Number(c.dataset.hwatuCanvas);
  const r = c.getBoundingClientRect();
  let data = canvasFrames.get(c);
  if (!data) {
    try { data = c.toDataURL('image/png'); } catch (e) { data = null; }
  }
  // WebGL without preserveDrawingBuffer yields blank data URLs (a few
  // hundred bytes). Record geometry either way: the driver falls back
  // to an engine-side screenshot crop for blank ones.
  canvases.push({
    i, data, w: Math.round(r.width), h: Math.round(r.height),
    doc_x: Math.round(r.left + scrollX), doc_y: Math.round(r.top + scrollY),
    blank: !data || data.length < 2000,
  });
});
// Lazy images: promote data-src/srcset and currentSrc to src.
document.querySelectorAll('img').forEach(img => {
  const cur = img.currentSrc || img.src;
  if (cur) img.setAttribute('src', cur);
  img.removeAttribute('srcset');
  img.removeAttribute('data-src');
  img.setAttribute('loading', 'eager');
});
// Videos: pin the poster (frame capture is out of scope for stills).
document.querySelectorAll('video').forEach(v => {
  v.removeAttribute('autoplay');
});

// 3. Asset manifest: everything referenced by the rendered page.
const assets = new Set();
const abs = (u) => { try { return new URL(u, location.href).href; } catch (e) { return null; } };
document.querySelectorAll('img[src]').forEach(el => { const u = abs(el.getAttribute('src')); if (u && !u.startsWith('data:')) assets.add(u); });
document.querySelectorAll('source[src]').forEach(el => { const u = abs(el.getAttribute('src')); if (u && !u.startsWith('data:')) assets.add(u); });
document.querySelectorAll('video[poster]').forEach(el => { const u = abs(el.getAttribute('poster')); if (u) assets.add(u); });
// url(...) references inside the collected CSS (images + fonts).


// Inner scroll positions (carousels, code panes). Restored by a tiny
// inline script the materializer appends, since scroll-snap can land
// the clone's scrollers on a different snap point.
// Two phases — read everything, then write the data attributes —
// because a dataset write invalidates style and turns the next
// element's layout read into a full recalc: interleaved, this loop is
// O(n²) on JS-heavy DOMs (scale.com-sized pages blow the eval budget).
const scrolls = [];
{
  const MAX_SCAN = 60000; // huge DOMs: cap the walk, report the truth below
  const all = document.querySelectorAll('*');
  const n = Math.min(all.length, MAX_SCAN);
  const found = [];
  for (let k = 0; k < n; k++) {
    const el = all[k];
    // Record every scrollable box, including ones at 0: scroll-snap in
    // the clone can otherwise pick a different initial snap point.
    const scrollable = el.scrollWidth > el.clientWidth + 4 || el.scrollHeight > el.clientHeight + 4;
    if (!scrollable || el === document.documentElement || el === document.body) continue;
    found.push([el, el.scrollLeft, el.scrollTop]);
  }
  found.forEach(([el, left, top], i) => {
    el.dataset.hwatuScroll = i;
    scrolls.push({ i, left, top });
  });
}

// 3.5 Pin transition state. Rules like `max-height: var(--max-height)`
// gated on JS-managed classes can resolve differently in the clone
// (hydration state, hover, mid-cycle carousels). For every element
// that declares a transition, bake the *rendered* values of the
// transitioned properties into inline style, so the clone shows the
// exact captured frame of every accordion/reveal/fade.
const PINNABLE = ['max-height', 'height', 'width', 'max-width', 'opacity', 'transform', 'visibility',
  'grid-template-rows', 'background-color', 'border-top-color', 'border-right-color',
  'border-bottom-color', 'border-left-color', 'color', 'box-shadow'];
// A transition on the shorthand ('background', 'border') covers its
// longhands, so match by prefix in both directions.
const covers = (declared, pin) => declared === 'all' || pin === declared || pin.startsWith(declared + '-');
// Pins are emitted as CSS rules keyed by data attribute, NOT inline
// styles: the materializer scopes them in a @media block matching the
// capture width, so other widths fall back to the site's own
// responsive CSS instead of wearing this width's measurements.
const pins = [];
{
  // Same two-phase discipline as the scroll walk above: all
  // getComputedStyle reads first, all dataset writes after, or the
  // style invalidation makes this O(n²) on large DOMs.
  const MAX_SCAN = 60000;
  const all = document.querySelectorAll('*');
  const n = Math.min(all.length, MAX_SCAN);
  const found = [];
  for (let k = 0; k < n; k++) {
    const el = all[k];
    // Scroll-effect subtrees are owned by the replay runtime (or the
    // baked entry state): pinning their computed values here would
    // cement one sweep instant with !important and fight the replay
    // script's inline writes (the pin-vs-normalizer ownership papercut
    // from the scale.com clone).
    if (el.closest && el.closest('[data-hwatu-scroll-effect]')) continue;
    const st = getComputedStyle(el);
    const hasTransition = st.transitionProperty && !st.transitionDuration.split(',').every(d => parseFloat(d) === 0);
    // CSS animation state does not survive serialization: the clone
    // restarts every cycle at load, so a rotating headline renders all
    // of its phrases overlapped. Pin the rendered values and kill the
    // animation instead — the still shows the captured instant.
    const hasAnimation = st.animationName && st.animationName !== 'none';
    if (!hasTransition && !hasAnimation) continue;
    const decls = [];
    let wanted;
    if (hasAnimation) {
      wanted = PINNABLE;
      decls.push('animation: none !important');
    } else {
      const props = st.transitionProperty.split(',').map(p => p.trim());
      wanted = PINNABLE.filter(p => props.some(d => covers(d, p)));
    }
    for (const p of wanted) {
      const v = st.getPropertyValue(p);
      if (v) decls.push(`${p}: ${v} !important`);
    }
    if (decls.length) found.push([el, decls.join('; ')]);
  }
  found.forEach(([el, css], i) => {
    el.dataset.hwatuPin = i;
    pins.push({ i, css });
  });
}

// Honest-report inputs: count what the static still drops. Scripts
// are stripped below by design; interactive-looking elements lose
// their handlers with them. The clone report names both.
const scriptCount = document.querySelectorAll('script').length;
const interactiveCount = document.querySelectorAll(
  'a[href], button, [role=button], [onclick], input, select, textarea, summary, [role=link], [role=tab], [role=menuitem]'
).length;

// Parser fixed-point guard: DOMs produced by JS/React can be valid live
// trees that are not representable as equivalent text/html. The common
// case is block-flow line wrappers inside <p>; when reparsed, the HTML
// parser auto-closes the paragraph and ejects the wrapper siblings. Rewrite
// only those unsafe paragraphs in the cloned document so serialization is a
// fixed point while preserving attributes, children, and paragraph semantics.
function parserFixedPointRewrite(doc) {
  const BLOCK_IN_P = new Set([
    'address', 'article', 'aside', 'blockquote', 'details', 'dialog', 'div',
    'dl', 'fieldset', 'figcaption', 'figure', 'footer', 'form', 'h1', 'h2',
    'h3', 'h4', 'h5', 'h6', 'header', 'hgroup', 'hr', 'main', 'menu', 'nav',
    'ol', 'p', 'pre', 'section', 'table', 'ul'
  ]);
  const hasBarePSelector = () => {
    const re = /(^|[^.#\w-])p(?=$|[^\w-])/i;
    for (const sheet of sheets) {
      const text = sheet.text || '';
      if (re.test(text)) return true;
    }
    return false;
  };
  const stats = {
    rewritten: [],
    reparsed_tag_count_deltas: [],
    injected_css: false,
  };
  let i = 0;
  for (const p of [...doc.querySelectorAll('p')]) {
    const offenders = [...p.children].filter(c => BLOCK_IN_P.has(c.tagName.toLowerCase()));
    if (!offenders.length) continue;
    const div = doc.ownerDocument.createElement('div');
    for (const attr of [...p.attributes]) div.setAttribute(attr.name, attr.value);
    div.setAttribute('role', div.getAttribute('role') || 'paragraph');
    div.setAttribute('data-hwatu-was', 'p');
    div.setAttribute('data-hwatu-parser-fix', String(i));
    while (p.firstChild) div.appendChild(p.firstChild);
    p.replaceWith(div);
    stats.rewritten.push({
      i,
      from: 'p',
      to: 'div',
      reason: 'block child inside p would be ejected by text/html reparse',
      block_children: [...new Set(offenders.map(c => c.tagName.toLowerCase()))],
      class: div.getAttribute('class') || null,
      id: div.getAttribute('id') || null,
    });
    i += 1;
  }
  if (stats.rewritten.length && hasBarePSelector()) {
    const style = doc.ownerDocument.createElement('style');
    style.setAttribute('data-hwatu-parser-fix-style', '');
    style.textContent = ':where([data-hwatu-was="p"][role="paragraph"]) { display: block; margin-block: 1em; }';
    (doc.querySelector('head') || doc).appendChild(style);
    stats.injected_css = true;
  }
  const html = '<!doctype html>\n' + doc.outerHTML;
  const reparsed = document.implementation.createHTMLDocument('hwatu-reparse-check');
  reparsed.open();
  reparsed.write(html);
  reparsed.close();
  const countTags = (root) => {
    const counts = new Map();
    for (const el of root.querySelectorAll('*')) {
      const tag = el.tagName.toLowerCase();
      counts.set(tag, (counts.get(tag) || 0) + 1);
    }
    return counts;
  };
  const liveCounts = countTags(doc);
  const reparsedCounts = countTags(reparsed.documentElement);
  const tags = new Set([...liveCounts.keys(), ...reparsedCounts.keys()]);
  for (const tag of [...tags].sort()) {
    const live = liveCounts.get(tag) || 0;
    const reparsed = reparsedCounts.get(tag) || 0;
    if (live !== reparsed) stats.reparsed_tag_count_deltas.push({ tag, live, reparsed });
  }
  stats.fixed_point = stats.reparsed_tag_count_deltas.length === 0;
  stats.html = html;
  return stats;
}

// 4. Serialized DOM. Strip scripts (the clone is a static still) and
//    live stylesheet links (we inline the CSSOM text instead).
const doc = document.documentElement.cloneNode(true);
// Static still: drop scripts (React re-hydration against a serialized
// post-render DOM tears the layout apart) and stylesheet links (CSSOM
// text is inlined instead).
doc.querySelectorAll('script, link[rel=stylesheet], link[rel=preload], link[rel=modulepreload]').forEach(el => el.remove());
doc.querySelectorAll('noscript').forEach(el => {
  // Render noscript content: our clone runs without JS.
  const span = document.createElement('div');
  span.innerHTML = el.textContent;
  el.replaceWith(span);
});

const parserFixedPoint = parserFixedPointRewrite(doc);

return {
  base: location.href,
  title: document.title,
  html: parserFixedPoint.html,
  parserFixedPoint: {
    rewritten: parserFixedPoint.rewritten,
    reparsed_tag_count_deltas: parserFixedPoint.reparsed_tag_count_deltas,
    fixed_point: parserFixedPoint.fixed_point,
    injected_css: parserFixedPoint.injected_css,
  },
  sheets,
  assets: [...assets].slice(0, 500),
    canvases,
    scrolls,
    pins,
    scrollEffects,
    scripts: scriptCount,
    interactive: interactiveCount,
    viewport: { w: innerWidth, h: innerHeight, dpr: devicePixelRatio },
  };
})();
