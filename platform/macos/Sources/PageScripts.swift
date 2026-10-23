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
let challengeJS = #"""
function detectHwatuChallenge() {
  const evidence = [];
  const add = (kind, detail, weight) => evidence.push({ kind, detail: String(detail).slice(0, 160), weight });
  const haystack = `${document.title || ''}\n${document.body ? document.body.innerText : ''}`.toLowerCase();
  const textChecks = [
    ['cloudflare', 'checking if the site connection is secure'],
    ['cloudflare', 'verify you are human'],
    ['generic', 'complete the security check'],
    ['generic', 'prove you are human'],
    ['generic', 'are you a robot'],
  ];
  for (const [kind, phrase] of textChecks) if (haystack.includes(phrase)) add(kind, phrase, 3);

  const nodes = [...document.querySelectorAll('iframe, script, div, input, textarea')];
  for (const el of nodes.slice(0, 400)) {
    const tag = el.tagName.toLowerCase();
    const attrs = ['src', 'title', 'name', 'id', 'class', 'data-sitekey', 'data-testid']
      .map(a => el.getAttribute(a) || '').join(' ').toLowerCase();
    if (!attrs) continue;
    if (attrs.includes('turnstile')) add('turnstile', `${tag} ${attrs}`, 4);
    if (attrs.includes('hcaptcha') || attrs.includes('h-captcha')) add('hcaptcha', `${tag} ${attrs}`, 4);
    if (attrs.includes('recaptcha') || attrs.includes('g-recaptcha')) add('recaptcha', `${tag} ${attrs}`, 4);
    if (attrs.includes('cf-challenge') || attrs.includes('challenge-platform')) add('cloudflare', `${tag} ${attrs}`, 4);
  }

  const weight = evidence.reduce((sum, e) => sum + e.weight, 0);
  const firstStrong = evidence.find(e => e.weight >= 4) || evidence[0];
  const challengeType = firstStrong ? firstStrong.kind : null;
  const confidence = Math.max(0, Math.min(1, weight / 8));
  const present = evidence.length > 0 && confidence >= 0.35;
  return {
    status: present ? 'challenge' : 'clear',
    challenge_type: present ? challengeType : null,
    confidence,
    evidence: evidence.slice(0, 12),
    actionable: present,
    manual_required: present,
    url: location.href,
    title: document.title || '',
  };
}
"""#
let motionJS = #"""
// Declared motion inventory for `motion`: a function body run by eval.
// getAnimations() for running CSS/WAAPI animations and transitions, plus a
// CSSOM walk for @keyframes and idle declared transitions/animations.
const MAX = 200;
const sel = (el) => {
  if (!el || !el.tagName) return null;
  let s = el.tagName.toLowerCase();
  if (el.id) return s + '#' + el.id;
  if (el.classList && el.classList.length) s += '.' + [...el.classList].slice(0, 3).join('.');
  return s;
};
const animations = document.getAnimations().slice(0, MAX).map(a => {
  const out = { state: a.playState };
  if (a.animationName) out.name = a.animationName;           // CSSAnimation
  if (a.transitionProperty) out.property = a.transitionProperty; // CSSTransition
  const ef = a.effect;
  if (ef) {
    const t = ef.getTiming();
    out.duration_ms = t.duration;
    out.delay_ms = t.delay;
    out.easing = t.easing;
    out.iterations = t.iterations;
    out.direction = t.direction;
    out.fill = t.fill;
    if (ef.target) out.target = sel(ef.target);
    try {
      out.keyframes = ef.getKeyframes().map(k => {
        const kf = {};
        for (const [key, v] of Object.entries(k)) {
          if (key === 'composite' || key === 'computedOffset') continue;
          kf[key] = v;
        }
        return kf;
      });
    } catch (e) { /* cross-origin effect */ }
  }
  out.current_time_ms = a.currentTime;
  return out;
});
// CSSOM: @keyframes rules + elements with declared transitions that
// are idle right now (they only show up in getAnimations mid-flight).
const keyframes = {};
const kfSeen = new Set();
for (const sheet of document.styleSheets) {
  let rules;
  try { rules = sheet.cssRules; } catch (e) { continue; } // cross-origin
  const walk = (list) => {
    for (const r of list) {
      if (r.type === CSSRule.KEYFRAMES_RULE && !kfSeen.has(r.name)) {
        kfSeen.add(r.name);
        keyframes[r.name] = [...r.cssRules].map(k => ({ offset: k.keyText, style: k.style.cssText }));
      } else if (r.cssRules) {
        walk(r.cssRules);
      }
    }
  };
  walk(rules);
}
const declared = [];
const els = document.querySelectorAll('*');
for (const el of els) {
  if (declared.length >= MAX) break;
  const st = getComputedStyle(el);
  const tp = st.transitionProperty;
  const td = st.transitionDuration;
  const hasTransition = tp && tp !== 'all' || (td && td.split(',').some(d => parseFloat(d) > 0));
  const hasAnimation = st.animationName && st.animationName !== 'none';
  if (!hasTransition && !hasAnimation) continue;
  const d = { target: sel(el) };
  if (hasTransition) {
    d.transition = {
      property: tp, duration: td,
      timing: st.transitionTimingFunction, delay: st.transitionDelay,
    };
  }
  if (hasAnimation) {
    d.animation = {
      name: st.animationName, duration: st.animationDuration,
      timing: st.animationTimingFunction, delay: st.animationDelay,
      iterations: st.animationIterationCount, direction: st.animationDirection,
      fill: st.animationFillMode, state: st.animationPlayState,
    };
  }
  declared.push(d);
}
return { animations, keyframes, declared };
"""#
