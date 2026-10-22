// GENERATED from page-scripts/irreversible.js. Do not edit by hand.
// Regenerate: scripts/gen-page-scripts.sh
let irreversibleJS = #"""
(function (el) {
  const control = el.closest('button, a, input, [role="button"], [role="menuitem"], [role="link"]') || el;
  const parts = [
    control.innerText || control.textContent || '',
    control.value || '',
    control.getAttribute('aria-label') || '',
    control.getAttribute('title') || '',
  ];
  const label = parts.join(' ').replace(/\s+/g, ' ').trim().slice(0, 160);
  const text = label.toLowerCase();
  const form = control.form || control.closest('form');
  const action = ((form && form.getAttribute('action')) || '').toLowerCase();
  const rules = [
    ['payment', /\b(place (your )?order|pay( now)?|complete (purchase|order|payment)|confirm (purchase|order|payment)|buy now|submit order|checkout and pay|purchase|subscribe now|start (my )?subscription|donate)\b/],
    ['send', /\b(send( now| message| email| money| payment)?|post( now)?|publish|submit application|transfer)\b/],
    ['delete', /\b(delete|remove (account|permanently)|permanently delete|erase|close account|cancel (my )?(account|subscription))\b/],
  ];
  for (const [kind, re] of rules) {
    if (re.test(text)) return { label, kind };
  }
  if (/\/(checkout|payment|purchase|order)\b.*\b(submit|place|confirm|complete)/.test(action)) {
    return { label, kind: 'payment' };
  }
  return null;
})(el)
"""#
