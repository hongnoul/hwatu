// `scroll`: one of selector, toY or byPages, then report the position.
// Runs after native-time.js. Inputs (consts defined by the caller):
// selector, nth, contains, toY, byPages.
if (selector !== null) {
  let els = [...document.querySelectorAll(selector)];
  const total = els.length;
  if (contains !== null)
    els = els.filter(e => ((e.textContent || '') + ' ' + (e.value || '')).includes(contains));
  const el = els[nth];
  if (!el) {
    const filt = contains === null ? '' : ` (${els.length} after contains filter)`;
    throw new Error(`no match: ${total} element(s) for selector${filt}, nth=${nth}`);
  }
  el.scrollIntoView({ block: 'center', behavior: 'instant' });
  var matched = {
    matches: els.length,
    tag: el.tagName.toLowerCase(),
    text: (el.textContent || '').trim().slice(0, 120),
  };
} else if (toY !== null) {
  window.scrollTo({ top: toY, behavior: 'instant' });
} else {
  window.scrollBy({ top: window.innerHeight * (byPages ?? 1.0), behavior: 'instant' });
}
// Let the instant scroll settle before measuring. Not rAF: frames
// never fire in headless/unmapped windows, which are exactly where
// agent scrolls run. scrollTo/scrollBy with behavior:'instant'
// update scrollY synchronously; one macrotask covers layout shifts.
// Native-clock sleep so a paused virtual clock cannot stall it.
await hwatuSleep(0);
// Headless views get no rendering opportunities, so pump the clock shim's
// tracked scroll listeners and IntersectionObservers after settling.
__hwatuNative?.pumpViewport?.();
const doc = document.documentElement;
const maxY = Math.max(0, doc.scrollHeight - window.innerHeight);
return {
  x: window.scrollX,
  y: window.scrollY,
  max_y: maxY,
  at_bottom: window.scrollY >= maxY - 1,
  ...(typeof matched === 'undefined' ? {} : { matched }),
};
