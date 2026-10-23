// Untrusted click for `click`: runs after native-time helpers and
// target.js, which leave `el` and `matched` in scope.
el.scrollIntoView({ block: 'center', behavior: 'instant' });
const r = el.getBoundingClientRect();
const x = r.left + r.width / 2, y = r.top + r.height / 2;
	const targetView = el.ownerDocument.defaultView || window;
	const opts = { bubbles: true, cancelable: true, composed: true, view: targetView,
	                clientX: x, clientY: y, button: 0, detail: 1 };
	el.dispatchEvent(new targetView.PointerEvent('pointerdown', { ...opts, pointerId: 1, isPrimary: true }));
	el.dispatchEvent(new targetView.MouseEvent('mousedown', opts));
	if (el.focus) el.focus();
	el.dispatchEvent(new targetView.PointerEvent('pointerup', { ...opts, pointerId: 1, isPrimary: true }));
	el.dispatchEvent(new targetView.MouseEvent('mouseup', opts));
	el.click ? el.click() : el.dispatchEvent(new targetView.MouseEvent('click', opts));
// Give a same-document reaction (SPA routing, form handlers) one
// macrotask to run, so the reported url reflects the click. Uses the
// native clock so a paused virtual clock cannot stall it.
await hwatuSleep(0);
return { clicked: matched, url: location.href };
