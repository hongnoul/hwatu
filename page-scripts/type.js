// Untrusted type for `type`: runs after native-time helpers and
// target.js. Inputs (consts defined by the caller): text, clear, enter.
el.scrollIntoView({ block: 'center', behavior: 'instant' });
if (el.focus) el.focus();
const fire = (t) => el.dispatchEvent(new Event(t, { bubbles: true }));
	const view = el.ownerDocument.defaultView || window;
	if (el instanceof view.HTMLSelectElement) {
  const opt = [...el.options].find(o => o.value === text || o.textContent.trim() === text);
  if (!opt) throw new Error(`no <option> matching ${JSON.stringify(text)} (values: ${[...el.options].map(o => o.value).slice(0, 20).join(', ')})`);
  el.value = opt.value;
  fire('input'); fire('change');
	} else if (el instanceof view.HTMLInputElement || el instanceof view.HTMLTextAreaElement) {
  // Use the element's own realm. A target adopted from an iframe can pass the
  // type check while rejecting a setter borrowed from the top-level realm.
	  const proto = el instanceof view.HTMLInputElement ? view.HTMLInputElement.prototype : view.HTMLTextAreaElement.prototype;
  const setter = Object.getOwnPropertyDescriptor(proto, 'value').set;
  setter.call(el, clear ? text : el.value + text);
  fire('input'); fire('change');
} else if (el.isContentEditable) {
  if (clear) {
    // Select the existing editor contents instead of mutating textContent.
    // Rich editors such as Google Sheets keep an internal model that only
    // observes editing commands; direct DOM clearing desynchronizes it and
    // causes the replacement text to be appended to the old value.
    const doc = el.ownerDocument;
    const selection = doc.getSelection();
    const range = doc.createRange();
    range.selectNodeContents(el);
    selection.removeAllRanges();
    selection.addRange(range);
  }
  el.dispatchEvent(new InputEvent('beforeinput', { bubbles: true, cancelable: true, inputType: 'insertText', data: text }));
  document.execCommand ? document.execCommand('insertText', false, text) : el.textContent += text;
  fire('input');
} else {
  throw new Error(`element <${el.tagName.toLowerCase()}> is not typeable (need input, textarea, select, or contenteditable)`);
}
if (enter) {
  const key = { bubbles: true, cancelable: true, key: 'Enter', code: 'Enter', keyCode: 13, which: 13 };
  const handled = !el.dispatchEvent(new KeyboardEvent('keydown', key));
  el.dispatchEvent(new KeyboardEvent('keypress', key));
  el.dispatchEvent(new KeyboardEvent('keyup', key));
  if (!handled && el.form) el.form.requestSubmit ? el.form.requestSubmit() : el.form.submit();
}
await hwatuSleep(0);
const value = el.value !== undefined ? el.value : el.textContent;
return { typed: matched, value: String(value).slice(0, 200), url: location.href };
