const c = window.__hwatu_clock;
if (!c) return { error: "virtual clock not installed in this page (reload it under this daemon build)" };
const OBS_MS = __OBS_MS__;
const TICK = 1000 / 60;
const MAX_TRACKS = 24;
const MAX_ELEMENTS = 4000;
const sel = (el) => {
  if (!el || !el.tagName) return null;
  let s = el.tagName.toLowerCase();
  if (el.id) return s + '#' + el.id;
  if (el.classList && el.classList.length) s += '.' + [...el.classList].slice(0, 3).join('.');
  return s;
};

// -- discovery: who moves? -------------------------------------------
// MutationObserver catches style/class writers (covers styles the
// rect diff below would miss, e.g. an element about to start moving);
// the rect diff over a 3-frame virtual step catches everything that
// actually moved, however it is driven (transform, layout, canvas
// containers repositioned by script).
const mutated = new Set();
const mo = new MutationObserver(recs => {
  for (const r of recs) {
    if (r.target && r.target.nodeType === 1) mutated.add(r.target);
  }
});
mo.observe(document.documentElement, { attributes: true, subtree: true, attributeFilter: ['style', 'class'] });

const all = [];
for (const el of document.querySelectorAll('body *')) {
  if (all.length >= MAX_ELEMENTS) break;
  const r = el.getBoundingClientRect();
  if (r.width > 2 && r.height > 2) { all.push([el, r.x, r.y]); }
}
c.pause();
// Real macrotask yield: React (and friends) schedule state updates on
// scheduler macrotasks, so an IO-gated animation ("start when in
// view") only arms after one. `await null` would only drain
// microtasks. MessageChannel, not setTimeout: hidden pages throttle
// DOM timers to ~1s, which would turn 30 yields into 30 real seconds.
const yieldReal = () => new Promise(r => {
  const ch = new MessageChannel();
  ch.port1.onmessage = () => r();
  ch.port2.postMessage(0);
});
c.step(TICK * 2);
await yieldReal(); // IO pump delivered; let gated starters arm
c.step(TICK * 3);
await yieldReal();

const movingSet = new Set();
for (const [el, x0, y0] of all) {
  const r = el.getBoundingClientRect();
  if (Math.abs(r.x - x0) > 0.5 || Math.abs(r.y - y0) > 0.5) movingSet.add(el);
}
// Movement is inherited: when a container moves, every descendant
// "moves". Track only topmost movers so the marquee is one track,
// not 37.
const topmost = [];
for (const el of movingSet) {
  let p = el.parentElement, top = true;
  while (p) { if (movingSet.has(p)) { top = false; break; } p = p.parentElement; }
  if (top) topmost.push(el);
}
// Style-mutated elements that have not moved yet may be one-shot
// movers about to fire; observe them too, movers first.
for (const el of mutated) {
  if (topmost.length >= MAX_TRACKS) break;
  if (!el.getBoundingClientRect || movingSet.has(el)) continue;
  let p = el.parentElement, covered = false;
  while (p) { if (movingSet.has(p)) { covered = true; break; } p = p.parentElement; }
  if (!covered) topmost.push(el);
}
mo.disconnect();

const tracks = topmost.slice(0, MAX_TRACKS).map(el => {
  const st = getComputedStyle(el);
  return {
    el,
    target: sel(el),
    property: st.transform && st.transform !== 'none' ? 'transform' : 'layout',
    scroll_w: el.scrollWidth, scroll_h: el.scrollHeight,
    xs: [], ys: [],
  };
});

// -- dense window: one sample per virtual frame ------------------------
const frames = Math.max(2, Math.min(Math.round(OBS_MS / TICK), 600));
for (let i = 0; i < frames; i++) {
  for (const t of tracks) {
    const r = t.el.getBoundingClientRect();
    t.xs.push(Math.round(r.x * 100) / 100);
    t.ys.push(Math.round(r.y * 100) / 100);
  }
  c.step(TICK);
  if (i % 5 === 4) await yieldReal(); else await null;
}

// -- wrap hunt: fast-forward to catch loop periods ---------------------
// A marquee's loop period is often minutes; nobody observes that in
// real time. Under virtual time we can step coarse chunks and watch
// for the position to jump against its own linear prediction.
const CHUNK = 100; // virtual ms between hunt samples
const HUNT_REAL_BUDGET_MS = 20000;
const huntStart = c.native.now();
let hunted = [];
for (const t of tracks) {
  for (const axis of ['x', 'y']) {
    const s = axis === 'x' ? t.xs : t.ys;
    const n = s.length;
    if (n < 8) continue;
    const deltas = [];
    for (let i = 1; i < n; i++) deltas.push(s[i] - s[i - 1]);
    const nz = deltas.filter(d => Math.abs(d) > 0.01);
    if (!nz.length) continue;
    const pos = nz.filter(d => d > 0).length;
    const mono = Math.max(pos, nz.length - pos) / nz.length;
    const v = (s[n - 1] - s[0]) / ((n - 1) * TICK); // px per virtual ms
    // Monotonic, still moving at the end: a loop candidate.
    const tail = Math.abs(s[n - 1] - s[n - 2]);
    if (mono >= 0.9 && Math.abs(v) * 1000 > 1 && tail > 0.01) {
      const extent = Math.max(t.scroll_w, t.scroll_h);
      const capMs = Math.min(200000, Math.max(20000, 2.5 * extent / Math.abs(v)));
      hunted.push({ t, axis, v, last: s[n - 1], capMs, wrap1: null, done: false });
      break; // one hunt axis per track
    }
  }
}
let vtime = 0;
while (hunted.some(h => !h.done) && c.native.now() - huntStart < HUNT_REAL_BUDGET_MS) {
  if (!hunted.some(h => !h.done && vtime < h.capMs)) break;
  c.step(CHUNK);
  vtime += CHUNK;
  await yieldReal();
  for (const h of hunted) {
    if (h.done) continue;
    if (vtime >= h.capMs) { h.done = true; continue; }
    const r = h.t.el.getBoundingClientRect();
    const p = h.axis === 'x' ? r.x : r.y;
    const pred = h.last + h.v * CHUNK;
    if (Math.abs(p - pred) > Math.max(24, Math.abs(h.v * CHUNK) * 4)) {
      // One wrap is enough: the fit derives the period from
      // jump/velocity. Confirming with a second wrap would double
      // the hunt for marginal precision.
      h.wrap1 = { t_ms: vtime, jump: p - pred };
      h.done = true;
    }
    h.last = p;
  }
}
for (const h of hunted) {
  h.t.wrap = { axis: h.axis, wrap1: h.wrap1 };
}
c.resume();
return {
  dt_ms: TICK,
  frames,
  window_ms: Math.round(frames * TICK),
  hunted_ms: vtime,
  tracks: tracks.map(t => ({
    target: t.target, property: t.property,
    scroll_w: t.scroll_w, scroll_h: t.scroll_h,
    xs: t.xs, ys: t.ys, wrap: t.wrap || null,
  })),
};
