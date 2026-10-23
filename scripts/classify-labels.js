// Run in the page by scripts/test-conformance.sh: classify each label as
// a <button> on the current page with the engine's irreversible guard.
// The label set and the classifier are substituted by the caller.
const labels = __HWATU_LABELS__;
const out = { stop_missed: [], go_stopped: [] };
const slot = document.getElementById('slot');
const run = (label) => {
  slot.innerHTML = '';
  const el = document.createElement('button');
  el.textContent = label;
  slot.appendChild(el);
  return __HWATU_CLASSIFY__;
};
for (const l of labels.stop) if (!run(l)) out.stop_missed.push(l);
for (const l of labels.go) if (run(l)) out.go_stopped.push(l);
return out;
