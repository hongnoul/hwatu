// Snapshot for `snapshot`: a function body run by eval. The caller
// replaces __HWATU_INCLUDE_RECTS__ with true or false. Interactables are
// remembered on window.__hwatu_refs for --ref targeting (target.js).
const MAX_TEXT = 4000;
const MAX_ELS = 120;
const INCLUDE_RECTS = __HWATU_INCLUDE_RECTS__;
const clip = (s, n) => {
  s = (s || '').replace(/\s+/g, ' ').trim();
  return s.length > n ? s.slice(0, n - 1) + '…' : s;
};
const visible = (el) => {
  const r = el.getBoundingClientRect();
  if (r.width === 0 && r.height === 0) return false;
  const st = getComputedStyle(el);
  return st.visibility !== 'hidden' && st.display !== 'none';
};
const label = (el) => {
  const aria = el.getAttribute('aria-label');
  if (aria) return aria;
  if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement) {
    if (el.labels && el.labels.length) return el.labels[0].textContent;
    return el.placeholder || el.name || el.value || '';
  }
  if (el instanceof HTMLImageElement) return el.alt;
  return el.textContent;
};
const sel = 'a[href], button, input, textarea, select, [role=button], [role=link], [role=tab], [role=menuitem], [role=checkbox], [contenteditable=""], [contenteditable=true], [onclick]';
const els = [...document.querySelectorAll(sel)].filter(visible).slice(0, MAX_ELS);
window.__hwatu_refs = els;
const interactables = els.map((el, i) => {
  const out = { ref: i, tag: el.tagName.toLowerCase() };
  if (INCLUDE_RECTS) {
    const r = el.getBoundingClientRect();
    out.rect = [r.x, r.y, r.width, r.height];
  }
  const text = clip(label(el), 80);
  if (text) out.text = text;
  if (el.id) out.id = el.id;
  if (el instanceof HTMLInputElement) {
    out.type = el.type;
    if (el.value && el.type !== 'password') out.value = clip(el.value, 40);
    if (el.checked) out.checked = true;
  }
  if (el.name) out.name = el.name;
  if (el instanceof HTMLAnchorElement && el.href) out.href = clip(el.href, 120);
  if (el.disabled) out.disabled = true;
  return out;
});
const doc = document.documentElement;
// Per-line text (spaces collapsed within a line, newlines kept):
// agents read it the same, and `snapshot --diff` can report the one
// line that changed instead of the whole blob.
const rawText = document.body ? document.body.innerText : '';
let text = rawText.split('\n')
  .map(l => l.replace(/\s+/g, ' ').trim())
  .filter(Boolean)
  .join('\n');
if (text.length > MAX_TEXT) text = text.slice(0, MAX_TEXT - 1) + '…';
return {
  url: location.href,
  title: document.title,
  text,
  interactables,
  scroll: {
    y: window.scrollY,
    max_y: Math.max(0, doc.scrollHeight - window.innerHeight),
  },
};
