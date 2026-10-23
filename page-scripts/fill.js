// `fill`: set several fields in one call. Inputs (consts defined by the
// caller): FIELDS ([{selector, nth, contains, ref, value, checked}]),
// SUBMIT (bool).
const findTarget = (f) => {
  if (f.ref !== null) {
    const refs = window.__hwatu_refs;
    if (!refs) throw new Error('no snapshot taken; run `hwatu snapshot` first or use selectors');
    const el = refs[f.ref];
    if (!el) throw new Error(`ref ${f.ref} out of range`);
    if (!el.isConnected) throw new Error(`ref ${f.ref} is no longer in the document`);
    return el;
  }
  const documents = [document];
  for (let i = 0; i < documents.length; i++) {
    for (const frame of documents[i].querySelectorAll('iframe,frame')) {
      try {
        const child = frame.contentDocument;
        if (child && !documents.includes(child)) documents.push(child);
      } catch (_) {}
    }
  }
  let els = documents.flatMap(doc => [...doc.querySelectorAll(f.selector)]);
  if (f.contains !== null)
    els = els.filter(e => ((e.textContent || '') + ' ' + (e.value || '')).includes(f.contains));
  const el = els[f.nth];
  if (!el) throw new Error(`no match for ${f.selector} (nth=${f.nth})`);
  return el;
};
const fire = (el, t) => el.dispatchEvent(new Event(t, { bubbles: true }));
const applied = [];
let lastEl = null;
for (let i = 0; i < FIELDS.length; i++) {
  const f = FIELDS[i];
  let el;
  try {
    el = findTarget(f);
  } catch (err) {
    return { filled: applied.length, total: FIELDS.length, applied,
              failed: { index: i, error: String(err.message || err) } };
  }
  const view = el.ownerDocument.defaultView || window;
  try {
    if (f.checked !== null) {
      if (!(el instanceof view.HTMLInputElement) || (el.type !== 'checkbox' && el.type !== 'radio'))
        throw new Error(`element <${el.tagName.toLowerCase()}> is not a checkbox/radio`);
      if (el.checked !== f.checked) {
        el.click();
        if (el.checked !== f.checked) {
          const setter = Object.getOwnPropertyDescriptor(view.HTMLInputElement.prototype, 'checked').set;
          setter.call(el, f.checked);
          fire(el, 'input'); fire(el, 'change');
        }
      }
      applied.push({ index: i, tag: el.tagName.toLowerCase(), checked: el.checked });
    } else if (el instanceof view.HTMLSelectElement) {
      const opt = [...el.options].find(o => o.value === f.value || o.textContent.trim() === f.value);
      if (!opt) throw new Error(`no <option> matching ${JSON.stringify(f.value)}`);
      el.value = opt.value;
      fire(el, 'input'); fire(el, 'change');
      applied.push({ index: i, tag: 'select', value: el.value });
    } else if (el instanceof view.HTMLInputElement || el instanceof view.HTMLTextAreaElement) {
      const proto = el instanceof view.HTMLInputElement
        ? view.HTMLInputElement.prototype : view.HTMLTextAreaElement.prototype;
      const setter = Object.getOwnPropertyDescriptor(proto, 'value').set;
      el.focus && el.focus();
      setter.call(el, f.value);
      fire(el, 'input'); fire(el, 'change');
      applied.push({ index: i, tag: el.tagName.toLowerCase(), value: String(el.value).slice(0, 100) });
    } else if (el.isContentEditable) {
      const doc = el.ownerDocument;
      const selection = doc.getSelection();
      const range = doc.createRange();
      range.selectNodeContents(el);
      selection.removeAllRanges();
      selection.addRange(range);
      el.dispatchEvent(new InputEvent('beforeinput', { bubbles: true, cancelable: true, inputType: 'insertText', data: f.value }));
      doc.execCommand ? doc.execCommand('insertText', false, f.value) : el.textContent = f.value;
      fire(el, 'input');
      applied.push({ index: i, tag: el.tagName.toLowerCase(), value: f.value.slice(0, 100) });
    } else {
      throw new Error(`element <${el.tagName.toLowerCase()}> is not fillable`);
    }
  } catch (err) {
    return { filled: applied.length, total: FIELDS.length, applied,
              failed: { index: i, error: String(err.message || err) } };
  }
  lastEl = el;
}
let submitted = false;
if (SUBMIT && lastEl && lastEl.form) {
  lastEl.form.requestSubmit ? lastEl.form.requestSubmit() : lastEl.form.submit();
  submitted = true;
}
return { filled: applied.length, total: FIELDS.length, applied, submitted, url: location.href };
