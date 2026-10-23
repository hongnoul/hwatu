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
  // A solved widget stays on the page. The widget writes its response
  // token into a named field on success; a non-empty token means solved.
  const tokenFields = {
    recaptcha: 'textarea[name="g-recaptcha-response"], #g-recaptcha-response',
    hcaptcha: 'textarea[name="h-captcha-response"], textarea[name="g-recaptcha-response"]',
    turnstile: 'input[name="cf-turnstile-response"]',
  };
  const tokenSel = tokenFields[challengeType];
  const tokenEls = tokenSel ? [...document.querySelectorAll(tokenSel)] : [];
  const solved = tokenEls.length > 0 && tokenEls.every(t => (t.value || '').length > 0);
  if (solved) evidence.push({ kind: challengeType, detail: 'response token present (solved)', weight: 0 });
  const present = evidence.length > 0 && confidence >= 0.35 && !solved;
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
