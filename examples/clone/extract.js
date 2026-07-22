// extract.js — runs inside the target page via `hwatu eval`.
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
document.querySelectorAll('canvas').forEach((c, i) => {
  try {
    c.dataset.hwatuCanvas = i;
    const r = c.getBoundingClientRect();
    canvases.push({ i, data: c.toDataURL('image/png'), w: Math.round(r.width), h: Math.round(r.height) });
  } catch (e) { /* tainted canvas */ }
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
const scrolls = [];
{
  let i = 0;
  for (const el of document.querySelectorAll('*')) {
    // Record every scrollable box, including ones at 0: scroll-snap in
    // the clone can otherwise pick a different initial snap point.
    const scrollable = el.scrollWidth > el.clientWidth + 4 || el.scrollHeight > el.clientHeight + 4;
    if (!scrollable || el === document.documentElement || el === document.body) continue;
    el.dataset.hwatuScroll = i;
    scrolls.push({ i, left: el.scrollLeft, top: el.scrollTop });
    i++;
  }
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
for (const el of document.querySelectorAll('*')) {
  const st = getComputedStyle(el);
  if (!st.transitionProperty || st.transitionDuration.split(',').every(d => parseFloat(d) === 0)) continue;
  const props = st.transitionProperty.split(',').map(p => p.trim());
  const wanted = PINNABLE.filter(p => props.some(d => covers(d, p)));
  for (const p of wanted) {
    const v = st.getPropertyValue(p);
    if (v) el.style.setProperty(p, v);
  }
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
  viewport: { w: innerWidth, h: innerHeight, dpr: devicePixelRatio },
};
