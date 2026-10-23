// One-shot `expect`: polls until the assertion holds or `deadline` ms
// pass. Runs after native-time.js and visibility.js. Inputs (consts
// defined by the caller): selector, nth, contains, wantText, absent,
// wantVisible, deadline.
const started = hwatuNow();
let previousVisibilitySignature = null;
let stableVisibility = null;

function check() {
  let els = [...document.querySelectorAll(selector)];
  const total = els.length;
  if (contains !== null)
    els = els.filter(e => ((e.textContent || '') + ' ' + (e.value || '')).includes(contains));
  const el = els[nth];
  if (absent) {
    if (els.length === 0) return { ok: true };
    const t = (els[0].textContent || '').trim().slice(0, 120);
    return { ok: false, why: `expected no match, found ${els.length} (first: <${els[0].tagName.toLowerCase()}> ${JSON.stringify(t)})` };
  }
  if (!el) {
    const filt = contains === null ? '' : ` (${els.length} after contains filter)`;
    return { ok: false, why: `no match: ${total} element(s) for selector${filt}, nth=${nth}` };
  }
  const actual = (el.textContent || '').trim();
  if (wantText !== null && !actual.includes(wantText)) {
    return { ok: false, why: `text mismatch: expected to contain ${JSON.stringify(wantText)}, got ${JSON.stringify(actual.slice(0, 200))}` };
  }
  if (wantVisible) {
    const visibility = hwatuInspectVisibility(el);
    if (visibility.why) {
      previousVisibilitySignature = null;
      return { ok: false, why: `element matched but is not visible: ${visibility.why}`, visibility };
    }
    if (previousVisibilitySignature !== visibility.signature) {
      previousVisibilitySignature = visibility.signature;
      return { ok: false, stabilizing: true, why: 'visibility has not remained stable across samples', visibility };
    }
    stableVisibility = visibility;
  }
  return {
    ok: true,
    matches: els.length,
    tag: el.tagName.toLowerCase(),
    text: actual.slice(0, 120),
    ...(wantVisible ? { visibility: stableVisibility } : {}),
  };
}

let result = check();
while (!result.ok && hwatuNow() - started < deadline) {
  await hwatuSleep(100);
  result = check();
}
result.elapsed_ms = hwatuNow() - started;
if (!result.ok) throw new Error(`expect failed after ${result.elapsed_ms} ms: ${result.why}`);
return result;
