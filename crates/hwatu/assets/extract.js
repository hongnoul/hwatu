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

return {
  base: location.href,
  title: document.title,
  html: '<!doctype html>\n' + doc.outerHTML,
  sheets,
  assets: [...assets].slice(0, 500),
  canvases,
  scrolls,
  pins,
  viewport: { w: innerWidth, h: innerHeight, dpr: devicePixelRatio },
};
})();
