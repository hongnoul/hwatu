// Page-local key press for `press`. The caller replaces __HWATU_KEY__
// with a JS string literal and prepends native-time.js.
const key = __HWATU_KEY__;
const describe = (el) => {
  if (!el || el.nodeType !== 1) return null;
  const out = { tag: el.tagName.toLowerCase() };
  if (el.id) out.id = el.id;
  const text = (el.getAttribute('aria-label') || el.textContent || el.value || '')
    .replace(/\s+/g, ' ').trim().slice(0, 120);
  if (text) out.text = text;
  return out;
};
const deepActive = () => {
  let active = document.activeElement;
  for (;;) {
    let nested = active && active.shadowRoot && active.shadowRoot.activeElement;
    if (!nested && active && (active.localName === 'iframe' || active.localName === 'frame')) {
      try {
        const child = active.contentDocument;
        if (child && child.activeElement && child.activeElement !== child.body)
          nested = child.activeElement;
      } catch (_) {}
    }
    if (!nested) return active;
    active = nested;
  }
};
const eventTarget = (active) => active || document.body || document.documentElement;
const dispatchKey = (target, type) => {
  const view = target.ownerDocument.defaultView || window;
  const code = key === 'Tab' ? 9
    : key === 'Escape' ? 27
    : key === 'ArrowLeft' ? 37
    : key === 'ArrowRight' ? 39
    : 13;
  return target.dispatchEvent(new view.KeyboardEvent(type, {
    bubbles: true, cancelable: true, composed: true,
    key, code: key, keyCode: code, which: code
  }));
};
if (key === 'Tab') {
  let sequence = 0;
  const stops = [];
  const tabbable = (el) => {
    const natural = el.matches(
      'a[href], area[href], button, input:not([type=hidden]), select, textarea, summary, iframe, frame, audio[controls], video[controls], [contenteditable=""], [contenteditable=true]'
    );
    if (!natural && !el.hasAttribute('tabindex')) return false;
    if (el.tabIndex < 0 || el.matches(':disabled') || el.closest('[inert]')) return false;
    const view = el.ownerDocument.defaultView || window;
    const style = view.getComputedStyle(el);
    return style.display !== 'none' && style.visibility !== 'hidden'
      && el.getClientRects().length > 0;
  };
  const visit = (root) => {
    const doc = root.nodeType === 9 ? root : root.ownerDocument;
    const walker = doc.createTreeWalker(root, 1);
    let el;
    while ((el = walker.nextNode())) {
      if (tabbable(el)) stops.push({ el, sequence: sequence++, tabIndex: el.tabIndex });
      if (el.shadowRoot) visit(el.shadowRoot);
      if (el.localName === 'iframe' || el.localName === 'frame') {
        try {
          if (el.contentDocument) visit(el.contentDocument);
        } catch (_) {}
      }
    }
  };
  visit(document);
  stops.sort((a, b) => {
    const ap = a.tabIndex > 0, bp = b.tabIndex > 0;
    if (ap !== bp) return ap ? -1 : 1;
    if (ap && a.tabIndex !== b.tabIndex) return a.tabIndex - b.tabIndex;
    return a.sequence - b.sequence;
  });
  const active = deepActive();
  const target = eventTarget(active);
  const allowed = dispatchKey(target, 'keydown');
  let focused = active;
  let wrapped = false;
  if (allowed && stops.length) {
    const current = stops.findIndex((stop) => stop.el === active);
    const next = (current + 1) % stops.length;
    wrapped = current === stops.length - 1;
    focused = stops[next].el;
    focused.focus();
  }
  dispatchKey(eventTarget(focused), 'keyup');
  return { pressed: key, focused: describe(focused), wrapped, canceled: !allowed };
}

if (key === 'Escape') {
  const active = deepActive();
  const target = eventTarget(active);
  const downAllowed = dispatchKey(target, 'keydown');
  dispatchKey(target, 'keyup');
  let dialog = null;
  let cancelAllowed = null;
  if (downAllowed) {
    const openDialogs = [...document.querySelectorAll('dialog[open]')];
    dialog = openDialogs.at(-1) || null;
    if (dialog) {
      const view = dialog.ownerDocument.defaultView || window;
      cancelAllowed = dialog.dispatchEvent(new view.Event('cancel', { cancelable: true }));
      if (cancelAllowed && dialog.open) dialog.close();
    }
  }
  await hwatuSleep(0);
  return {
    pressed: key,
    focused: describe(deepActive()),
    dialogClosed: Boolean(dialog) && !dialog.open,
    canceled: !downAllowed || cancelAllowed === false,
    url: location.href
  };
}

if (key === 'ArrowLeft' || key === 'ArrowRight') {
  const active = deepActive();
  const target = eventTarget(active);
  const downAllowed = dispatchKey(target, 'keydown');
  const before = active && active.scrollLeft;
  let scrolled = false;
  if (downAllowed && active && active.scrollWidth > active.clientWidth) {
    const direction = key === 'ArrowLeft' ? -1 : 1;
    active.scrollBy({ left: direction * 40, behavior: 'instant' });
    scrolled = active.scrollLeft !== before;
  }
  dispatchKey(target, 'keyup');
  await hwatuSleep(0);
  return {
    pressed: key,
    focused: describe(deepActive()),
    scrolled,
    scrollLeft: active && active.scrollLeft,
    canceled: !downAllowed,
    url: location.href
  };
}

const active = deepActive();
const target = eventTarget(active);
const downAllowed = dispatchKey(target, 'keydown');
if (downAllowed) dispatchKey(target, 'keypress');
dispatchKey(target, 'keyup');
let activated = false;
let submitted = false;
if (downAllowed && active) {
  const activatable = active.matches(
    'a[href], button, summary, input[type=button], input[type=submit], input[type=reset], input[type=checkbox], input[type=radio], [role=button], [role=link], [onclick]'
  );
  if (activatable && typeof active.click === 'function') {
    active.click();
    activated = true;
  } else {
    const view = active.ownerDocument.defaultView || window;
    if (active instanceof view.HTMLInputElement && active.form) {
      active.form.requestSubmit();
      submitted = true;
    }
  }
}
await hwatuSleep(0);
return {
  pressed: key,
  focused: describe(deepActive()),
  activated,
  submitted,
  canceled: !downAllowed,
  url: location.href
};
