// Resolve the target element for click/type/content and friends.
// Inputs (consts defined by the caller): selector, nth, contains, refIdx.
// Outputs: `el` (the element) and `matched` (what was hit, for replies).
let el;
if (refIdx !== null) {
  const refs = window.__hwatu_refs;
  if (!refs) throw new Error('no snapshot taken; run `hwatu snapshot` first or use a selector');
  el = refs[refIdx];
  if (!el) throw new Error(`ref ${refIdx} out of range (snapshot had ${refs.length} interactables)`);
  if (!el.isConnected) throw new Error(`ref ${refIdx} is no longer in the document; re-run snapshot`);
  var matched = { ref: refIdx };
	} else {
	  // ATS portals such as iCIMS commonly place the actual application in a
	  // same-origin iframe under a branded outer page. Traverse all accessible
	  // frame documents so selectors retain their normal meaning there. Frames
	  // with a different origin are skipped by the browser's same-origin guard.
	  const documents = [document];
	  for (let i = 0; i < documents.length; i++) {
	    for (const frame of documents[i].querySelectorAll('iframe,frame')) {
	      try {
	        const child = frame.contentDocument;
	        if (child && !documents.includes(child)) documents.push(child);
	      } catch (_) {}
	    }
	  }
	  let els = documents.flatMap(doc => [...doc.querySelectorAll(selector)]);
  const total = els.length;
  if (contains !== null) {
    els = els.filter(e => ((e.textContent || '') + ' ' + (e.value || '')).includes(contains));
    // Leaf preference: a page-root div's textContent contains
    // everything, so ancestor matches shadow the element the caller
    // actually means. Keep only matches that do not contain another
    // match; document order is preserved, so nth stays meaningful
    // over the de-shadowed list.
    els = els.filter(e => !els.some(o => o !== e && e.contains(o)));
  }
  el = els[nth];
  if (!el) {
    const filt = contains === null ? '' : ` (${els.length} after contains filter)`;
    throw new Error(`no match: ${total} element(s) for selector${filt}, nth=${nth}`);
  }
  var matched = { matches: els.length };
}
matched.tag = el.tagName.toLowerCase();
matched.text = (el.textContent || el.value || '').trim().slice(0, 120);
