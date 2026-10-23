// GENERATED from page-scripts/. Do not edit by hand.
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
  // Generic final buttons ("Finish", "Confirm", "Submit", "Complete") are
  // irreversible on a final checkout review: the URL names checkout or
  // payment, and the page shows a grand total.
  if (/^(finish|confirm|submit|complete|done|place)\b/.test(text)) {
    const where = (location.pathname + ' ' + document.title).toLowerCase();
    const page = (document.body ? document.body.innerText : '').toLowerCase();
    const atCheckout = /checkout|payment|purchase|order|review/.test(where);
    const hasTotal = /\b(order )?total\b[^\n]{0,40}[$€£¥₩]\s?\d|[$€£¥₩]\s?\d[^\n]{0,40}\btotal\b|\btotal:?\s*[$€£¥₩]?\s?\d/.test(page);
    if (atCheckout && hasTotal) return { label, kind: 'payment' };
  }
  return null;
})(el)
"""#
let loginWallJS = #"""
(function () {
  const visible = e => {
    const r = e.getBoundingClientRect();
    const s = getComputedStyle(e);
    return r.width > 0 && r.height > 0 && s.visibility !== 'hidden' && s.display !== 'none';
  };
  const pw = [...document.querySelectorAll('input[type="password"]')].filter(visible).length;
  const otp = [...document.querySelectorAll('input[autocomplete="one-time-code"], input[name*="otp" i], input[name*="code" i][inputmode="numeric"]')].filter(visible).length;
  const text = (document.title + ' ' + (document.body ? document.body.innerText.slice(0, 4000) : '')).toLowerCase();
  const signals = [];
  if (/\b(sign in|log in|login)\b/.test(text)) signals.push('sign_in_text');
  if (/\b(verification code|one-time code|enter the code)\b/.test(text)) signals.push('code_text');
  if (/\/(login|signin|sign-in|auth|sso)\b/i.test(location.pathname)) signals.push('login_path');
  if (pw === 0 && otp === 0 && !signals.includes('login_path')) return null;
  return { password_fields: pw, otp_fields: otp, signals };
})()
"""#
