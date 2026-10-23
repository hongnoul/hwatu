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
  // Phrases are matched as whole words. JS \b is ASCII-only, so word
  // boundaries use Unicode letters; CJK labels are short and matched as
  // substrings. Add a language by adding its phrases; keep phrases
  // specific to the final action (not "cart", "next", "continue").
  const W = (alts) => new RegExp('(?<![\\p{L}\\p{N}])(?:' + alts.join('|') + ')(?![\\p{L}\\p{N}])', 'u');
  const rules = [
    ['payment', W([
      // en
      'place (?:your )?order', 'pay(?: now)?', 'complete (?:purchase|order|payment)',
      'confirm (?:purchase|order|payment)', 'buy now', 'submit order', 'checkout and pay',
      'purchase', 'subscribe now', 'start (?:my )?subscription', 'donate',
      // de
      'zahlungspflichtig bestellen', 'jetzt kaufen', 'kaufen', 'bestellung absenden',
      'jetzt bezahlen', 'bezahlen', 'kostenpflichtig bestellen', 'kauf abschließen',
      'bestellung abschließen',
      // fr
      'commander', 'payer(?: maintenant)?', 'valider (?:la |ma )?commande', 'acheter(?: maintenant)?',
      'confirmer (?:la |ma )?commande', 'passer (?:la |ma )?commande',
      // es / pt / it
      'realizar pedido', 'pagar(?: ahora| agora)?', 'comprar(?: ahora| agora)?', 'finalizar (?:compra|pedido)',
      'confirmar (?:compra|pedido)', 'concluir (?:compra|pedido)', 'fazer pedido',
      'acquista(?: ora)?', 'paga(?: ora)?', 'conferma ordine',
      // nl / ru
      'bestellen en betalen', 'nu kopen', 'afrekenen en betalen', 'bestelling plaatsen',
      'nu betalen', 'оформить заказ', 'оплатить',
      'купить(?: сейчас)?', 'подтвердить заказ',
    ])],
    ['payment', /注文を確定|注文する|購入する|今すぐ購入|支払う|^購入$|提交订单|确认订单|立即购买|^购买$|立即支付|确认支付|去付款|결제하기|주문하기|구매하기|주문 결제|^결제$/u],
    ['send', W([
      'send(?: now| message| email| money| payment)?', 'post(?: now)?', 'publish',
      'submit application', 'transfer',
      'senden', 'absenden', 'veröffentlichen', 'envoyer', 'publier', 'enviar', 'publicar',
      'invia', 'pubblica', 'verzenden', 'versturen', 'отправить', 'опубликовать',
    ])],
    ['send', /^(?:送信|发送|發送|보내기|전송)$/u],
    ['delete', W([
      'delete', 'remove (?:account|permanently)', 'permanently delete', 'erase', 'close account',
      'cancel (?:my )?(?:account|subscription)',
      'löschen', 'endgültig löschen', 'konto löschen', 'supprimer', 'eliminar', 'excluir',
      'elimina', 'verwijderen', 'удалить', 'borrar(?: cuenta)?', 'effacer', 'apagar(?: conta)?',
      'konto entfernen', 'cancella(?: account)?', 'excluir conta',
    ])],
    ['delete', /削除|删除|刪除|삭제/u],
  ];
  // "Send feedback", "Post code" and similar compound nouns are not the
  // final action.
  // Labels that name a section or setting rather than doing the action
  // ("Purchase history", "Payment method", "Send to a friend? Learn more").
  const notFinal = /(?<![\p{L}])(?:send feedback|send to a friend|postal code|post code|feedback senden|history|historique|historial|verlauf|method|methods|details|settings|learn more|again|resend|erneut|à nouveau|de nuevo|novamente)(?![\p{L}])/u;
  // Navigation toward a step ("Continue to payment", "Weiter zur Zahlung",
  // "購入手続きへ") moves to the next page; the final action comes later.
  const toward = /(?<![\p{L}])(?:continue to|proceed to|go to|weiter zu[rm]?|zur|zum|continuer vers|passer au|continuar (?:al|a|para)|ir a|ir para|procedi|verder naar|перейти к)(?![\p{L}])|(?:手続きへ|へ進む|に進む|へ$|前往|继续|繼續|(?:으)?로 이동)/u;
  // A delete is irreversible whatever it deletes ("Delete history",
  // "Delete payment method"). Payment and send labels that name a section
  // or repeat an earlier step are not the final action.
  const skipPaySend = notFinal.test(text) || toward.test(text);
  for (const [kind, re] of rules) {
    if (kind !== 'delete' && skipPaySend) continue;
    if (re.test(text)) return { label, kind };
  }
  if (/\/(checkout|payment|purchase|order)\b.*\b(submit|place|confirm|complete)/.test(action)) {
    return { label, kind: 'payment' };
  }
  // Checkout pages, any language. On a page that looks like checkout (the
  // URL or title names it, or a total with a currency is shown), a
  // button whose short label is built on a pay/order/buy stem is treated
  // as the final action. A missed stop places an order; a false stop
  // costs one confirmation, so recall wins here. Off checkout pages only
  // the explicit phrases above apply.
  const where = (location.pathname + ' ' + document.title).toLowerCase();
  const page = (document.body ? document.body.innerText : '').toLowerCase();
  const currency = '[$€£¥₩₹₽]|\\b(?:usd|eur|gbp|jpy|krw|cny|rub|chf|brl)\\b';
  const totalWord = '(?:total|gesamt|summe|montant|importe|totale|totaal|итого|合計|合计|总计|총액|합계)';
  const hasTotal = new RegExp(totalWord + '[^\\n]{0,40}(?:' + currency + ')\\s?\\d|(?:' + currency + ')\\s?\\d[^\\n]{0,40}' + totalWord + '|' + totalWord + ':?\\s*(?:' + currency + ')?\\s?\\d', 'u').test(page);
  const atCheckout = /checkout|payment|purchase|order|review|kasse|bestell|commande|paiement|pedido|pago|ordine|pagamento|bestelling|заказ|оплат|注文|購入|订单|支付|결제|주문/u.test(where);
  const buttonLike = control.tagName === 'BUTTON' || control.getAttribute('role') === 'button'
    || (control.tagName === 'INPUT' && /^(?:submit|button)$/.test(control.type || ''));
  if (buttonLike && (atCheckout || hasTotal) && !skipPaySend) {
    const words = text.split(/[\s,.;:!?()]+/u).filter(Boolean);
    const payStem = /^(?:pay|zahl|bezahl|payer|paiement|pagar|pago|pagamento|paga|betal|betaal|afreken|оплат|order|bestell|command|pedido|ordine|bestelling|заказ|buy|kauf|achet|compr|acquist|koop|куп|покуп|checkout|kostenpflichtig|zahlungspflichtig|abschließ|abonn|subscri)/u;
    const cjkPay = /付款|支付|決済|支払|결제|注文|订单|訂單|下单|下單|주문|購入|购买|購買|구매/u;
    // A bare noun ("Payment", "Orders", "Zahlungsarten") or a label that
    // names a list, method or guide is a section, not the action.
    const section = new RegExp([
      // possessives and navigation verbs: "My orders", "Track order", "View order"
      '^(?:my|mein|meine|meus|minhas|mes|mon|mis|i miei|le mie|mijn|мои|мой)\\s',
      '(?<![\\p{L}])(?:view|track|change|edit|manage|ver|cambiar|voir|modifier|ändern|anzeigen|verfolgen|rastrear|alterar|modifica|bekijk|изменить|посмотреть)(?![\\p{L}])',
      // section nouns, whole words
      '(?<![\\p{L}])(?:options?|summary|status|guide|help|hilfe|aide|ayuda|ajuda|aiuto|newsletter|methods?|formas|info|history|filters?|updates?|comments?|details?|historique|détails|detalles|detalhes|dettagli|resumen|résumé|riepilogo|способы|история|статус)(?![\\p{L}])',
      // German compounds end in the section noun: Zahlungsarten, Bestellübersicht
      '(?:arten|übersicht|verlauf|status|methoden|details|informationen|hilfe)(?![\\p{L}])',
      // CJK section nouns
      '方法|方式|수단|履歴|历史|詳細|详情|내역|状況|状态|상태',
      // a bare noun is a heading or tab, not an action
      '^(?:payment|payments|orders?|zahlung|zahlungen|bestellungen|commandes|pedidos|ordini|bestellingen|заказы|оплата)$',
    ].join('|'), 'u');
    if (words.length <= 5 && !section.test(text) && (words.some(w => payStem.test(w)) || cjkPay.test(text))) {
      return { label, kind: 'payment' };
    }
  }
  // Generic final buttons ("Finish", "Confirm", "Submit", "Complete") are
  // irreversible on a final checkout review: the URL names checkout or
  // payment, and the page shows a grand total.
  if (/^(finish|confirm|submit|complete|done|place)\b/.test(text) && atCheckout && hasTotal) {
    return { label, kind: 'payment' };
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
let targetJS = #"""
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
"""#
let snapshotJS = #"""
// Snapshot for `snapshot`: a function body run by eval. The caller
// replaces __HWATU_INCLUDE_RECTS__ with true or false. Interactables are
// remembered on window.__hwatu_refs for --ref targeting (target.js).
const MAX_TEXT = 4000;
const MAX_ELS = 120;
const INCLUDE_RECTS = __HWATU_INCLUDE_RECTS__;
const clip = (s, n) => {
  s = (s || '').replace(/\s+/g, ' ').trim();
  return s.length > n ? s.slice(0, n - 1) + '…' : s;
};
const visible = (el) => {
  const r = el.getBoundingClientRect();
  if (r.width === 0 && r.height === 0) return false;
  const st = getComputedStyle(el);
  return st.visibility !== 'hidden' && st.display !== 'none';
};
const label = (el) => {
  const aria = el.getAttribute('aria-label');
  if (aria) return aria;
  if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement) {
    if (el.labels && el.labels.length) return el.labels[0].textContent;
    return el.placeholder || el.name || el.value || '';
  }
  if (el instanceof HTMLImageElement) return el.alt;
  return el.textContent;
};
const sel = 'a[href], button, input, textarea, select, [role=button], [role=link], [role=tab], [role=menuitem], [role=checkbox], [contenteditable=""], [contenteditable=true], [onclick]';
const els = [...document.querySelectorAll(sel)].filter(visible).slice(0, MAX_ELS);
window.__hwatu_refs = els;
const interactables = els.map((el, i) => {
  const out = { ref: i, tag: el.tagName.toLowerCase() };
  if (INCLUDE_RECTS) {
    const r = el.getBoundingClientRect();
    out.rect = [r.x, r.y, r.width, r.height];
  }
  const text = clip(label(el), 80);
  if (text) out.text = text;
  if (el.id) out.id = el.id;
  if (el instanceof HTMLInputElement) {
    out.type = el.type;
    if (el.value && el.type !== 'password') out.value = clip(el.value, 40);
    if (el.checked) out.checked = true;
  }
  if (el.name) out.name = el.name;
  if (el instanceof HTMLAnchorElement && el.href) out.href = clip(el.href, 120);
  if (el.disabled) out.disabled = true;
  return out;
});
const doc = document.documentElement;
// Per-line text (spaces collapsed within a line, newlines kept):
// agents read it the same, and `snapshot --diff` can report the one
// line that changed instead of the whole blob.
const rawText = document.body ? document.body.innerText : '';
let text = rawText.split('\n')
  .map(l => l.replace(/\s+/g, ' ').trim())
  .filter(Boolean)
  .join('\n');
if (text.length > MAX_TEXT) text = text.slice(0, MAX_TEXT - 1) + '…';
return {
  url: location.href,
  title: document.title,
  text,
  interactables,
  scroll: {
    y: window.scrollY,
    max_y: Math.max(0, doc.scrollHeight - window.innerHeight),
  },
};
"""#
let contentJS = #"""
// Page content for `content`: a function body run by eval.
// Inputs (consts defined by the caller): MAX (char cap), FORMAT
// ('text' | 'html' | 'title' | 'links'), root (element to read).
if (FORMAT === 'title') {
  return { format: 'title', url: location.href, title: document.title };
}
if (FORMAT === 'html') {
  const html = root.outerHTML || '';
  return { format: 'html', url: location.href, title: document.title,
           truncated: html.length > MAX, content: html.slice(0, MAX) };
}
if (FORMAT === 'links') {
  const seen = new Set();
  const links = [];
  for (const a of root.querySelectorAll('a[href]')) {
    if (links.length >= 200) break;
    const href = a.href;
    if (!href || href.startsWith('javascript:') || seen.has(href)) continue;
    const text = (a.innerText || a.getAttribute('aria-label') || '').replace(/\s+/g, ' ').trim().slice(0, 120);
    seen.add(href);
    links.push({ url: href, text });
  }
  return { format: 'links', url: location.href, title: document.title, links };
}
const text = (root.innerText || root.textContent || '').replace(/[ \t]+/g, ' ').replace(/\n{3,}/g, '\n\n').trim();
return { format: 'text', url: location.href, title: document.title,
         truncated: text.length > MAX, content: text.slice(0, MAX) };
"""#
let clickJS = #"""
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
"""#
let typeJS = #"""
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
  if (!handled && el.form) {
    el.form.requestSubmit ? el.form.requestSubmit() : el.form.submit();
    await hwatuAwaitUnload(500);
  }
}
await hwatuSleep(0);
const value = el.value !== undefined ? el.value : el.textContent;
return { typed: matched, value: String(value).slice(0, 200), url: location.href };
"""#
let nativeTimeJS = #"""
// hwatuSleep/hwatuNow on the native clock, so a paused virtual clock
// (`hwatu clock pause`) cannot stall automation waits.
const __hwatuNative = (window.__hwatu_clock && window.__hwatu_clock.native) || null;
const hwatuSleep = (ms) => new Promise((r) =>
  __hwatuNative ? __hwatuNative.setTimeout(r, ms) : setTimeout(r, ms));
const hwatuNow = () => (__hwatuNative ? __hwatuNative.dateNow() : Date.now());
// After a form submit, wait for the navigation the submit queued, so the
// reply reports it instead of answering before WebKit starts it (Linux: 1
// in 60 `type --enter` replies came first, and a following `wait-load`
// returned on the old page). Once the page starts unloading, keep waiting:
// the commit destroys this context and the daemon replies `navigated`
// after the load. If nothing unloads within `ms` (a JS-handled submit),
// return. A navigation that never commits (204, cancelled) returns after
// 10 s.
const hwatuAwaitUnload = (ms) => new Promise((r) => {
  const later = __hwatuNative ? __hwatuNative.setTimeout : setTimeout;
  let unloading = false;
  const start = () => { unloading = true; later(r, 10000); };
  window.addEventListener('beforeunload', start, { once: true });
  window.addEventListener('pagehide', start, { once: true });
  later(() => { if (!unloading) r(); }, ms);
});
"""#
let pressJS = #"""
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
      await hwatuAwaitUnload(500);
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
"""#
let visibilityJS = #"""
// hwatuInspectVisibility(el): rendered-state rules shared by `expect
// --visible` and `expect --watch`.
function hwatuRoundedRect(r) {
  return {
    left: Math.round(r.left * 100) / 100,
    top: Math.round(r.top * 100) / 100,
    width: Math.round(r.width * 100) / 100,
    height: Math.round(r.height * 100) / 100,
  };
}

function hwatuRectChanged(a, b) {
  return ['left', 'top', 'width', 'height'].some(k => Math.abs(a[k] - b[k]) > 0.5);
}

// Inspect an element as a user would see it while leaving the caller's scroll
// position intact. Scrolling can still cause irreversible page work (lazy
// loading), so geometry/document mutations are returned as instability rather
// than silently becoming the basis of a passing assertion.
function hwatuInspectVisibility(el) {
  let effectiveOpacity = 1;
  const opacityChain = [];
  for (let n = el; n instanceof Element; n = n.parentElement) {
    const st = getComputedStyle(n);
    if (st.display === 'none') return { why: n === el ? 'display:none' : 'display:none on ancestor <' + n.tagName.toLowerCase() + '>' };
    if (st.visibility === 'hidden' || st.visibility === 'collapse')
      return { why: 'visibility:' + st.visibility + (n === el ? '' : ' on ancestor <' + n.tagName.toLowerCase() + '>') };
    const opacity = Number.parseFloat(st.opacity);
    if (Number.isFinite(opacity)) {
      effectiveOpacity *= opacity;
      if (opacity < 1) opacityChain.push({ tag: n.tagName.toLowerCase(), id: n.id || null, opacity });
    }
  }
  effectiveOpacity = Math.round(effectiveOpacity * 1000000) / 1000000;
  if (effectiveOpacity <= 0)
    return { why: 'effective opacity is 0', effective_opacity: effectiveOpacity, opacity_chain: opacityChain };

  const root = document.documentElement;
  const ancestorScroll = [];
  for (let n = el.parentElement; n instanceof Element; n = n.parentElement) {
    ancestorScroll.push({ node: n, left: n.scrollLeft, top: n.scrollTop });
  }
  const original = {
    x: scrollX,
    y: scrollY,
    width: root.scrollWidth,
    height: root.scrollHeight,
    rect: hwatuRoundedRect(el.getBoundingClientRect()),
  };
  if (original.rect.width === 0 || original.rect.height === 0)
    return { why: `zero-size box (${original.rect.width}x${original.rect.height})`, effective_opacity: effectiveOpacity, opacity_chain: opacityChain };

  let r = el.getBoundingClientRect();
  let scrolled = false;
  let why = null;
  let points = [];
  try {
    if (r.bottom <= 0 || r.top >= innerHeight || r.right <= 0 || r.left >= innerWidth) {
      scrolled = true;
      el.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
      r = el.getBoundingClientRect();
    }
    const left = Math.max(r.left, 0), right = Math.min(r.right, innerWidth);
    const top = Math.max(r.top, 0), bottom = Math.min(r.bottom, innerHeight);
    if (right <= left || bottom <= top) {
      why = `outside viewport after scroll (rect ${Math.round(r.left)},${Math.round(r.top)} ${Math.round(r.width)}x${Math.round(r.height)})`;
    } else {
      const ix = Math.min(1, (right - left) / 2), iy = Math.min(1, (bottom - top) / 2);
      const samples = [
        ['center', (left + right) / 2, (top + bottom) / 2],
        ['top-left', left + ix, top + iy], ['top-right', right - ix, top + iy],
        ['bottom-left', left + ix, bottom - iy], ['bottom-right', right - ix, bottom - iy],
      ];
      for (const [name, x, y] of samples) {
        const hit = document.elementFromPoint(x, y);
        const hitName = hit ? `${hit.tagName.toLowerCase()}${hit.id ? '#' + hit.id : ''}` : null;
        points.push({ name, x: Math.round(x * 100) / 100, y: Math.round(y * 100) / 100, hit: hitName });
        if (hit && (hit === el || el.contains(hit) || hit.contains(el)
            || (hit.shadowRoot && hit.shadowRoot.contains(el))
            || (hit instanceof HTMLLabelElement && hit.control === el))) continue;
        if (!hit) {
          why = `${name} point hits nothing (elementFromPoint returned null)`;
        } else {
          const t = (hit.textContent || '').trim().slice(0, 60);
          why = `${name} point covered by <${hitName}> ${JSON.stringify(t)}`;
        }
        break;
      }
    }
  } finally {
    if (scrolled) window.scrollTo(original.x, original.y);
    for (const saved of ancestorScroll) {
      saved.node.scrollLeft = saved.left;
      saved.node.scrollTop = saved.top;
    }
  }

  const restored = {
    x: scrollX,
    y: scrollY,
    width: root.scrollWidth,
    height: root.scrollHeight,
    rect: hwatuRoundedRect(el.getBoundingClientRect()),
  };
  const layoutChanged = original.width !== restored.width
    || original.height !== restored.height
    || Math.abs(original.x - restored.x) > 0.5
    || Math.abs(original.y - restored.y) > 0.5
    || hwatuRectChanged(original.rect, restored.rect);
  const ancestorScrollRestored = ancestorScroll.every(saved =>
    Math.abs(saved.node.scrollLeft - saved.left) <= 0.5
      && Math.abs(saved.node.scrollTop - saved.top) <= 0.5);
  const diagnostics = {
    scrolled,
    scroll_restored: Math.abs(original.x - restored.x) <= 0.5
      && Math.abs(original.y - restored.y) <= 0.5 && ancestorScrollRestored,
    ancestor_scroll_restored: ancestorScrollRestored,
    layout_changed: layoutChanged,
    effective_opacity: effectiveOpacity,
    opacity_chain: opacityChain,
    original_rect: original.rect,
    restored_rect: restored.rect,
    document_before: { width: original.width, height: original.height },
    document_after: { width: restored.width, height: restored.height },
    points,
  };
  if (layoutChanged && !why) why = 'visibility inspection changed page layout or target geometry';
  diagnostics.why = why;
  const signature = JSON.stringify({
    effective_opacity: effectiveOpacity,
    rect: original.rect,
    document: diagnostics.document_after,
    points,
    why,
  });
  Object.defineProperty(diagnostics, 'signature', { value: signature, enumerable: false });
  return diagnostics;
}
"""#
let expectJS = #"""
// One-shot `expect`: polls until the assertion holds or `deadline` ms
// pass. Runs after native-time.js and visibility.js. Inputs (consts
// defined by the caller): selector, nth, contains, wantText, absent,
// wantVisible, deadline.
const started = hwatuNow();
let previousVisibilitySignature = null;
let stableVisibility = null;

function check() {
  let els = [...document.querySelectorAll(selector)];
  const total = els.length;
  if (contains !== null)
    els = els.filter(e => ((e.textContent || '') + ' ' + (e.value || '')).includes(contains));
  const el = els[nth];
  if (absent) {
    if (els.length === 0) return { ok: true };
    const t = (els[0].textContent || '').trim().slice(0, 120);
    return { ok: false, why: `expected no match, found ${els.length} (first: <${els[0].tagName.toLowerCase()}> ${JSON.stringify(t)})` };
  }
  if (!el) {
    const filt = contains === null ? '' : ` (${els.length} after contains filter)`;
    return { ok: false, why: `no match: ${total} element(s) for selector${filt}, nth=${nth}` };
  }
  const actual = (el.textContent || '').trim();
  if (wantText !== null && !actual.includes(wantText)) {
    return { ok: false, why: `text mismatch: expected to contain ${JSON.stringify(wantText)}, got ${JSON.stringify(actual.slice(0, 200))}` };
  }
  if (wantVisible) {
    const visibility = hwatuInspectVisibility(el);
    if (visibility.why) {
      previousVisibilitySignature = null;
      return { ok: false, why: `element matched but is not visible: ${visibility.why}`, visibility };
    }
    if (previousVisibilitySignature !== visibility.signature) {
      previousVisibilitySignature = visibility.signature;
      return { ok: false, stabilizing: true, why: 'visibility has not remained stable across samples', visibility };
    }
    stableVisibility = visibility;
  }
  return {
    ok: true,
    matches: els.length,
    tag: el.tagName.toLowerCase(),
    text: actual.slice(0, 120),
    ...(wantVisible ? { visibility: stableVisibility } : {}),
  };
}

let result = check();
while (!result.ok && hwatuNow() - started < deadline) {
  await hwatuSleep(100);
  result = check();
}
result.elapsed_ms = hwatuNow() - started;
if (!result.ok) throw new Error(`expect failed after ${result.elapsed_ms} ms: ${result.why}`);
return result;
"""#
let scrollJS = #"""
// `scroll`: one of selector, toY or byPages, then report the position.
// Runs after native-time.js. Inputs (consts defined by the caller):
// selector, nth, contains, toY, byPages.
if (selector !== null) {
  let els = [...document.querySelectorAll(selector)];
  const total = els.length;
  if (contains !== null)
    els = els.filter(e => ((e.textContent || '') + ' ' + (e.value || '')).includes(contains));
  const el = els[nth];
  if (!el) {
    const filt = contains === null ? '' : ` (${els.length} after contains filter)`;
    throw new Error(`no match: ${total} element(s) for selector${filt}, nth=${nth}`);
  }
  el.scrollIntoView({ block: 'center', behavior: 'instant' });
  var matched = {
    matches: els.length,
    tag: el.tagName.toLowerCase(),
    text: (el.textContent || '').trim().slice(0, 120),
  };
} else if (toY !== null) {
  window.scrollTo({ top: toY, behavior: 'instant' });
} else {
  window.scrollBy({ top: window.innerHeight * (byPages ?? 1.0), behavior: 'instant' });
}
// Let the instant scroll settle before measuring. Not rAF: frames
// never fire in headless/unmapped windows, which are exactly where
// agent scrolls run. scrollTo/scrollBy with behavior:'instant'
// update scrollY synchronously; one macrotask covers layout shifts.
// Native-clock sleep so a paused virtual clock cannot stall it.
await hwatuSleep(0);
// Headless views get no rendering opportunities, so pump the clock shim's
// tracked scroll listeners and IntersectionObservers after settling.
__hwatuNative?.pumpViewport?.();
const doc = document.documentElement;
const maxY = Math.max(0, doc.scrollHeight - window.innerHeight);
return {
  x: window.scrollX,
  y: window.scrollY,
  max_y: maxY,
  at_bottom: window.scrollY >= maxY - 1,
  ...(typeof matched === 'undefined' ? {} : { matched }),
};
"""#
let fillJS = #"""
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
  await hwatuAwaitUnload(500);
}
return { filled: applied.length, total: FIELDS.length, applied, submitted, url: location.href };
"""#
let authContextJS = #"""
// `auth-context`: cookie and storage inventory (names only, never
// values) plus the login-wall probe. The caller substitutes the login-wall
// placeholder below with page-scripts/login-wall.js.
const out = { url: location.href, origin: location.origin };
try {
  const names = document.cookie ? document.cookie.split('; ').map(c => c.split('=')[0]).filter(Boolean) : [];
  out.cookie_names = names.slice(0, 50);
  out.cookie_count = names.length;
} catch (_) { out.cookie_count = 0; out.cookie_names = []; }
if (window.cookieStore && cookieStore.getAll) {
  try {
    const all = await cookieStore.getAll();
    out.cookie_count = all.length;
    out.cookie_names = all.map(c => c.name).slice(0, 50);
    const now = Date.now();
    out.persistent_cookies = all.filter(c => c.expires && c.expires > now).length;
    out.session_cookies = all.filter(c => !c.expires).length;
  } catch (_) {}
}
try {
  out.local_storage_keys = Object.keys(localStorage).slice(0, 50);
  out.local_storage_count = localStorage.length;
} catch (_) { out.local_storage_count = 0; }
try { out.session_storage_count = sessionStorage.length; } catch (_) { out.session_storage_count = 0; }
out.likely_authenticated = out.cookie_count > 0
  && (out.cookie_names || []).some(n => /sess|token|auth|sid|login|jwt|csrf/i.test(n));
out.login_wall = __LOGIN_WALL__;
return out;
"""#
let trustedResolveJS = #"""
// Installed in every frame at document start. Resolves a trusted-input
// target (selector, nth, contains, ref) in the top document or, through
// postMessage, in any child frame including cross-origin ones, and
// returns its centre in top-viewport coordinates. Also records trusted
// mousemoves for pointer calibration.
(() => {
  if (window.__hwatuTrustedResolveInstalled) return;
  window.__hwatuTrustedResolveInstalled = true;

  // ---- trusted pointer observation (calibration channel) ----
  // Every frame records trusted mousemoves. Child frames forward theirs
  // to the parent, which translates into its own viewport coordinates
  // and re-forwards, so the top frame stores a unified observation.
  const isTop = (() => { try { return window === window.top; } catch (_) { return false; } })();
  window.__hwatuMoveSeq = 0;
  const recordMove = (x, y) => {
    if (isTop) {
      window.__hwatuLastTrustedMove = { x, y, seq: ++window.__hwatuMoveSeq };
    } else {
      try { parent.postMessage({ __hwatuTrustedMove: true, x, y }, '*'); } catch (_) {}
    }
  };
  document.addEventListener('mousemove', (event) => {
    if (event.isTrusted) recordMove(event.clientX, event.clientY);
  }, { capture: true, passive: true });
  window.addEventListener('message', (event) => {
    const msg = event.data;
    if (!msg || msg.__hwatuTrustedMove !== true) return;
    const frame = [...document.querySelectorAll('iframe')]
      .find(f => f.contentWindow === event.source);
    if (!frame) return;
    const r = frame.getBoundingClientRect();
    recordMove(msg.x + r.left, msg.y + r.top);
  }, true);

  // ---- selector resolution across frames ----
  const ownText = (el) => ((el.textContent || '') + ' ' + (el.value || '')).trim();
  const select = (selector, nth, contains, refIdx) => {
    let el;
    let total = 0;
    let matches = 0;
    if (refIdx !== null && refIdx !== undefined) {
      const refs = window.__hwatu_refs;
      if (!refs) return null;
      el = refs[refIdx];
      total = refs.length;
      matches = refs.length;
      if (!el || !el.isConnected) return null;
    } else {
      let els;
      try { els = [...document.querySelectorAll(selector)]; }
      catch (_) { return null; }
      total = els.length;
      if (contains !== null && contains !== undefined)
        els = els.filter(e => ownText(e).includes(contains));
      matches = els.length;
      el = els[nth || 0];
      if (!el) return null;
    }
    try { el.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' }); } catch (_) {}
    const r = el.getBoundingClientRect();
    if (!Number.isFinite(r.left) || !Number.isFinite(r.top) || r.width <= 0 || r.height <= 0)
      return null;
    return {
      x: r.left + r.width / 2,
      y: r.top + r.height / 2,
      width: r.width,
      height: r.height,
      viewportWidth: window.innerWidth,
      viewportHeight: window.innerHeight,
      frameUrl: location.href,
      matched: {
        matches,
        total,
        tag: el.tagName ? el.tagName.toLowerCase() : '',
        text: ownText(el).slice(0, 120)
      }
    };
  };

  window.addEventListener('message', (event) => {
    const req = event.data;
    if (!req || req.__hwatuTrustedResolve !== true) return;
    const found = select(req.selector, req.nth, req.contains, req.refIdx);
    if (!found) return;
    event.source && event.source.postMessage({
      __hwatuTrustedResolveReply: true,
      token: req.token,
      found
    }, '*');
  }, true);

  window.__hwatuTrustedResolve = (selector, nth, contains, refIdx) => new Promise((resolve, reject) => {
    const local = select(selector, nth, contains, refIdx);
    if (local) {
      local.frame = 'top';
      resolve(local);
      return;
    }

    const frames = [...document.querySelectorAll('iframe')]
      .filter(frame => frame && frame.contentWindow);
    if (!frames.length) {
      reject(new Error(`no match for trusted selector ${JSON.stringify(selector)}`));
      return;
    }

    const token = `hwatu:${Date.now()}:${Math.random()}`;
    const done = (value, error) => {
      clearTimeout(timer);
      window.removeEventListener('message', onReply, true);
      error ? reject(error) : resolve(value);
    };
    const onReply = (event) => {
      const msg = event.data;
      if (!msg || msg.__hwatuTrustedResolveReply !== true || msg.token !== token || !msg.found)
        return;
      const frame = frames.find(f => f.contentWindow === event.source);
      if (!frame) return;
      const fr = frame.getBoundingClientRect();
      const found = msg.found;
      found.x += fr.left;
      found.y += fr.top;
      // x/y are now top-viewport coordinates; report the top viewport
      // size too so range checks stay coherent.
      found.viewportWidth = window.innerWidth;
      found.viewportHeight = window.innerHeight;
      found.frame = 'iframe';
      found.iframe = { id: frame.id || '', src: frame.src || '', x: fr.left, y: fr.top, width: fr.width, height: fr.height };
      done(found, null);
    };
    const timer = setTimeout(() => done(null, new Error(`no match for trusted selector ${JSON.stringify(selector)} in top document or child frames`)), 1000);
    window.addEventListener('message', onReply, true);
    const req = { __hwatuTrustedResolve: true, token, selector, nth, contains, refIdx };
    for (const frame of frames) frame.contentWindow.postMessage(req, '*');
  });
})();
"""#
let clockJS = #"""
(() => {
  if (window.__hwatu_clock) return;
  const g = window;
  const realPerf = performance;
  const realNow = performance.now.bind(performance);
  const NativeDate = g.Date;
  const nativeSetTimeout = g.setTimeout.bind(g);
  const nativeClearTimeout = g.clearTimeout.bind(g);
  const nativeSetInterval = g.setInterval.bind(g);
  const nativeClearInterval = g.clearInterval.bind(g);
  const nativeRaf = g.requestAnimationFrame.bind(g);
  const nativeCaf = g.cancelAnimationFrame.bind(g);
  // Wall-clock epoch at performance-timeline zero. In deterministic
  // start-paused mode the epoch is pinned to a constant: pages that
  // derive visible state from absolute Date.now() (live counters)
  // would otherwise differ across loads in their last digits even
  // with identical virtual timelines.
  const startPausedEpoch = 1700000000000;
  const configuredEpoch = Number(g.__hwatu_clock_epoch_ms);
  const dateBase = Number.isFinite(configuredEpoch)
    ? configuredEpoch
    : (g.__hwatu_clock_start_paused ? startPausedEpoch : (NativeDate.now() - realNow()));

  // Virtual timeline: continuous with the performance timeline until
  // the first pause (vnow() === realNow() while never paused).
  //
  // Deterministic-load mode (`HWATU_CLOCK_START_PAUSED` on the
  // daemon, surfaced as a pre-injected global): the clock installs
  // already paused at virtual t=0, so the *whole load* happens at one
  // frozen instant. Every timer deadline and every now() read during
  // parsing is then identical across loads, which is what makes two
  // fresh loads of the same page byte-comparable under `clock step`.
  const startPaused = !!g.__hwatu_clock_start_paused;
  let paused = startPaused;
  let vbase = startPaused ? 0 : realNow(); // virtual time at the last sync point
  let rbase = realNow();                   // realNow() at the last sync point
  const vnow = () => (paused ? vbase : vbase + (realNow() - rbase));

  // ---- timers: one registry, native delegation while live ---------
  // Entry: { cb, args, deadline (virtual ms), interval (ms|null),
  //          native (native timer id | null) }
  const timers = new Map();
  let nextTimerId = 1_000_000_000; // far from native ids
  const armNative = (id, entry) => {
    const delay = Math.max(0, entry.deadline - vnow());
    entry.native = nativeSetTimeout(() => fireTimer(id), delay);
  };
  const fireTimer = (id) => {
    const entry = timers.get(id);
    if (!entry) return;
    if (entry.interval !== null) {
      entry.deadline += entry.interval;
      entry.native = null;
      if (!paused) armNative(id, entry);
    } else {
      timers.delete(id);
    }
    try { entry.cb(...entry.args); } catch (e) { nativeSetTimeout(() => { throw e; }, 0); }
  };
  const schedule = (cb, delay, interval, args) => {
    const id = nextTimerId++;
    const entry = {
      cb, args,
      deadline: vnow() + Math.max(Number(delay) || 0, 0),
      interval, native: null,
    };
    timers.set(id, entry);
    if (!paused) armNative(id, entry);
    return id;
  };
  g.setTimeout = function (cb, delay, ...args) {
    if (typeof cb !== 'function') return nativeSetTimeout(cb, delay, ...args);
    return schedule(cb, delay, null, args);
  };
  g.setInterval = function (cb, delay, ...args) {
    if (typeof cb !== 'function') return nativeSetInterval(cb, delay, ...args);
    const ms = Math.max(Number(delay) || 0, 1);
    return schedule(cb, ms, ms, args);
  };
  const clearAny = (id) => {
    const entry = timers.get(id);
    if (entry) {
      if (entry.native !== null) nativeClearTimeout(entry.native);
      timers.delete(id);
    }
  };
  g.clearTimeout = (id) => { clearAny(id); nativeClearTimeout(id); };
  g.clearInterval = (id) => { clearAny(id); nativeClearInterval(id); };

  // ---- requestAnimationFrame: registry + pump ----------------------
  const rafCallbacks = new Map();
  let nextRafId = 1;
  let rafPump = null; // native rAF id driving the live flush
  const flushRaf = () => {
    const batch = [...rafCallbacks.values()];
    rafCallbacks.clear();
    const ts = vnow();
    for (const cb of batch) {
      try { cb(ts); } catch (e) { nativeSetTimeout(() => { throw e; }, 0); }
    }
  };
  const armPump = () => {
    if (paused || rafPump !== null || rafCallbacks.size === 0) return;
    rafPump = nativeRaf(() => { rafPump = null; flushRaf(); armPump(); });
  };
  g.requestAnimationFrame = (cb) => {
    const id = nextRafId++;
    rafCallbacks.set(id, cb);
    armPump();
    return id;
  };
  g.cancelAnimationFrame = (id) => { rafCallbacks.delete(id); };

  // ---- clocks the page reads ---------------------------------------
  realPerf.now = () => vnow();
  const virtualDateNow = () => Math.round(dateBase + vnow());
  // Date.now alone is insufficient: sites commonly choose visible
  // day/night variants with `new Date().getHours()`. A Proxy preserves
  // Date's native prototype, parse/UTC statics, subclass construction,
  // and explicit-argument parsing while virtualizing only the implicit
  // "now" paths (`Date()` and `new Date()`).
  const VirtualDate = new Proxy(NativeDate, {
    apply() {
      return new NativeDate(virtualDateNow()).toString();
    },
    construct(target, args, newTarget) {
      return Reflect.construct(
        target,
        args.length === 0 ? [virtualDateNow()] : args,
        newTarget,
      );
    },
    get(target, property, receiver) {
      if (property === 'now') return virtualDateNow;
      return Reflect.get(target, property, receiver);
    },
  });
  g.Date = VirtualDate;

  // ---- Math.random: optional seeded determinism ----------------------
  // Math.random is the one visible entropy source the virtual clock
  // does not cover: a page that renders from it can never be
  // byte-compared across loads. seedRandom(n) replaces it with
  // mulberry32 seeded from n, so same seed + same virtual timeline
  // (which fixes the *order* of random() calls) => identical
  // sequences. Default behavior is untouched native Math.random.
  const nativeRandom = Math.random.bind(Math);
  let randomSeed = null;
  const seedRandom = (n) => {
    n = Number(n);
    if (!Number.isFinite(n) || n < 0) return { error: 'seed needs a finite number >= 0' };
    randomSeed = n;
    // Fold a u64-ish seed into 32 bits, then mulberry32. >>> 0 keeps
    // arithmetic in uint32 land; the fold keeps distinct u64 seeds
    // from trivially colliding in the low word.
    let state = ((n >>> 0) ^ Math.floor(n / 4294967296)) >>> 0;
    Math.random = () => {
      state = (state + 0x6D2B79F5) >>> 0;
      let t = state;
      t = Math.imul(t ^ (t >>> 15), t | 1);
      t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
    return status();
  };
  // ---- declarative animations share the timeline --------------------
  // Animations we paused (vs. paused by the page itself): only these
  // are resumed by `resume`.
  const pausedByUs = new WeakSet();
  const pauseAnimations = () => {
    for (const a of document.getAnimations()) {
      try {
        if (a.playState === 'running') { a.pause(); pausedByUs.add(a); }
      } catch (e) { /* infinite timeline etc. */ }
    }
  };
  const advanceAnimations = (ms) => {
    for (const a of document.getAnimations()) {
      try {
        if (a.playState === 'running') { a.pause(); pausedByUs.add(a); }
        if (a.currentTime !== null) a.currentTime = Number(a.currentTime) + ms;
      } catch (e) { /* not seekable */ }
    }
  };

  // ---- IntersectionObserver: pumped under virtual stepping ----------
  // WebKit delivers IO callbacks on rendering opportunities, and a
  // hidden/unmapped (headless) page never gets one, so script gated on
  // "am I in view" (lazy loads, marquee starts) never runs headless.
  // Track registrations and, on each `step` tick, compute viewport
  // intersections manually and deliver entries with virtual timestamps.
  // While live, the native observer delivers as usual; the pump only
  // fires under step, and only when the observed state changed.
  const NativeIO = g.IntersectionObserver;
  const ioInstances = new Set();
  if (NativeIO) {
    g.IntersectionObserver = class IntersectionObserver extends NativeIO {
      constructor(cb, options) {
        const state = { cb, options: options || {}, targets: new Set(), last: new WeakMap() };
        super((entries, observer) => {
          // Keep the manual headless pump in sync with native delivery so a
          // rendering opportunity immediately before a pump cannot invoke the
          // page callback twice for the same state.
          for (const entry of entries) state.last.set(entry.target, entry.isIntersecting);
          cb(entries, observer);
        }, options);
        this.__hwatu = state;
        ioInstances.add(this);
      }
      observe(t) { this.__hwatu.targets.add(t); return super.observe(t); }
      unobserve(t) { this.__hwatu.targets.delete(t); return super.unobserve(t); }
      disconnect() { this.__hwatu.targets.clear(); ioInstances.delete(this); return super.disconnect(); }
    };
  }
  const ioRect = (root) => {
    if (root && root.getBoundingClientRect) return root.getBoundingClientRect();
    return { left: 0, top: 0, right: g.innerWidth, bottom: g.innerHeight,
             width: g.innerWidth, height: g.innerHeight };
  };
  const pumpIO = () => {
    for (const io of ioInstances) {
      const state = io.__hwatu;
      const rb = ioRect(state.options.root);
      const entries = [];
      for (const target of state.targets) {
        if (!target.isConnected) continue;
        const r = target.getBoundingClientRect();
        const ileft = Math.max(r.left, rb.left), itop = Math.max(r.top, rb.top);
        const iright = Math.min(r.right, rb.right), ibottom = Math.min(r.bottom, rb.bottom);
        const iw = Math.max(0, iright - ileft), ih = Math.max(0, ibottom - itop);
        const area = r.width * r.height;
        const ratio = area > 0 ? (iw * ih) / area : (iw >= 0 && ih >= 0 ? 1 : 0);
        const intersecting = iw > 0 && ih > 0;
        const last = state.last.get(target);
        if (last !== undefined && last === intersecting) continue;
        state.last.set(target, intersecting);
        entries.push({
          target, isIntersecting: intersecting, intersectionRatio: ratio,
          boundingClientRect: r,
          intersectionRect: { left: ileft, top: itop, width: iw, height: ih },
          rootBounds: rb, time: vnow(),
        });
      }
      if (entries.length) {
        try { state.cb(entries, io); } catch (e) { nativeSetTimeout(() => { throw e; }, 0); }
      }
    }
  };

  // ---- scroll events: pumped under virtual stepping ------------------
  // WebKit dispatches `scroll` events on rendering opportunities, so a
  // headless/unmapped page updates window.scrollY synchronously on
  // scrollTo() but never fires the event. Scroll-driven libraries
  // (Lenis, GSAP ScrollTrigger) hang off that event and never notice
  // the move. On each `step` tick, compare the window scroll offset
  // against the last delivered value and dispatch a plain `scroll`
  // event on document (bubbling to window) when it changed. Element
  // scrollers are not tracked: agent scrolls drive the window.
  let lastScrollX = g.scrollX;
  let lastScrollY = g.scrollY;
  const syncScroll = () => {
    lastScrollX = g.scrollX;
    lastScrollY = g.scrollY;
  };
  // Native scroll delivery, when WebKit does get a rendering opportunity,
  // wins over the fallback pump and suppresses a duplicate synthetic event.
  document.addEventListener('scroll', syncScroll, { capture: true });
  const pumpScroll = () => {
    if (g.scrollX === lastScrollX && g.scrollY === lastScrollY) return;
    lastScrollX = g.scrollX;
    lastScrollY = g.scrollY;
    try {
      document.dispatchEvent(new Event('scroll', { bubbles: true }));
    } catch (e) {}
  };
  const pumpViewport = () => {
    pumpScroll();
    pumpIO();
  };

  // ---- control surface ----------------------------------------------
  const status = () => ({
    installed: true,
    paused,
    virtual_ms: Math.round(vnow() * 1000) / 1000,
    pending_timers: timers.size,
    pending_rafs: rafCallbacks.size,
    seed: randomSeed,
  });
  const pause = () => {
    if (paused) return status();
    vbase = vnow();
    paused = true;
    for (const entry of timers.values()) {
      if (entry.native !== null) { nativeClearTimeout(entry.native); entry.native = null; }
    }
    if (rafPump !== null) { nativeCaf(rafPump); rafPump = null; }
    pauseAnimations();
    return status();
  };
  const resume = () => {
    if (!paused) return status();
    paused = false;
    rbase = realNow();
    for (const [id, entry] of timers) armNative(id, entry);
    armPump();
    for (const a of document.getAnimations()) {
      try { if (pausedByUs.has(a)) { pausedByUs.delete(a); a.play(); } } catch (e) {}
    }
    return status();
  };
  const TICK = 1000 / 60;
  const MAX_TIMER_FIRES_PER_TICK = 1000;
  const step = (ms) => {
    ms = Number(ms);
    if (!Number.isFinite(ms) || ms < 0) return { error: 'step needs a finite ms >= 0' };
    pause();
    let remaining = ms;
    while (remaining > 1e-9) {
      const dt = Math.min(TICK, remaining);
      remaining -= dt;
      vbase += dt;
      // Fire due timers, earliest first; intervals may refire within
      // the tick, capped so a 0 ms interval cannot hang the page.
      for (let fired = 0; fired < MAX_TIMER_FIRES_PER_TICK; fired++) {
        let dueId = null;
        let dueAt = Infinity;
        for (const [id, entry] of timers) {
          if (entry.deadline <= vbase + 1e-9 && entry.deadline < dueAt) {
            dueAt = entry.deadline;
            dueId = id;
          }
        }
        if (dueId === null) break;
        fireTimer(dueId);
      }
      pumpIO();
      pumpScroll();
      flushRaf();
      // Force a synchronous style recalc so CSS transitions triggered
      // by the timers/rAF above are *created* inside this tick, then
      // advance declarative animations by the same dt. Leaving the
      // advance to the end of the step let transition creation race
      // the engine's real-time render cycle: an animation born after
      // the loop finished missed its advance entirely, which showed
      // up as one-frame phase flicker between two lockstep windows.
      void document.documentElement && document.documentElement.offsetWidth;
      advanceAnimations(dt);
    }
    return status();
  };
  const set = (ms) => {
    ms = Number(ms);
    if (!Number.isFinite(ms) || ms < 0) return { error: 'set needs a finite ms >= 0' };
    const cur = vnow();
    const wasPaused = paused;
    pause();
    if (ms < cur - 1e-6) {
      // The first control operation on a live page may establish a new
      // virtual epoch after navigation/font/hydration waits have settled.
      // Preserve each timer's remaining delay while rebasing now() to the
      // requested value. Once a page is already paused, backwards travel
      // remains unsupported because arbitrary script state cannot rewind.
      if (!wasPaused) {
        const delta = ms - cur;
        for (const entry of timers.values()) entry.deadline += delta;
        vbase = ms;
        return status();
      }
      return { error: `cannot go backwards: virtual time is ${Math.round(cur)} ms, requested ${ms} ms`, ...status() };
    }
    return step(ms - cur);
  };
  // A pre-injected flag (set by the daemon before this script when a
  // seed was requested for future loads) seeds from document start,
  // before any page script can capture or consume native Math.random.
  if (typeof g.__hwatu_clock_seed === 'number') seedRandom(g.__hwatu_clock_seed);
  window.__hwatu_clock = {
    pause, resume, step, set, status, seedRandom,
    nativeRandom,
    // Native escape hatch for the harness's own plumbing: hwatu's
    // scroll/click/type/expect/challenge JS must keep running in real
    // time while the *page's* clocks are frozen, or pausing the page
    // would deadlock the tool driving it.
    native: {
      setTimeout: nativeSetTimeout,
      clearTimeout: nativeClearTimeout,
      now: () => realNow(),
      dateNow: () => Math.round(dateBase + realNow()),
      pumpViewport,
    },
  };
})();
"""#
let motionObserveJS = #"""
const c = window.__hwatu_clock;
if (!c) return { error: "virtual clock not installed in this page (reload it under this daemon build)" };
const OBS_MS = __OBS_MS__;
const TICK = 1000 / 60;
const MAX_TRACKS = 24;
const MAX_ELEMENTS = 4000;
const sel = (el) => {
  if (!el || !el.tagName) return null;
  let s = el.tagName.toLowerCase();
  if (el.id) return s + '#' + el.id;
  if (el.classList && el.classList.length) s += '.' + [...el.classList].slice(0, 3).join('.');
  return s;
};

// -- discovery: who moves? -------------------------------------------
// MutationObserver catches style/class writers (covers styles the
// rect diff below would miss, e.g. an element about to start moving);
// the rect diff over a 3-frame virtual step catches everything that
// actually moved, however it is driven (transform, layout, canvas
// containers repositioned by script).
const mutated = new Set();
const mo = new MutationObserver(recs => {
  for (const r of recs) {
    if (r.target && r.target.nodeType === 1) mutated.add(r.target);
  }
});
mo.observe(document.documentElement, { attributes: true, subtree: true, attributeFilter: ['style', 'class'] });

const all = [];
for (const el of document.querySelectorAll('body *')) {
  if (all.length >= MAX_ELEMENTS) break;
  const r = el.getBoundingClientRect();
  if (r.width > 2 && r.height > 2) { all.push([el, r.x, r.y]); }
}
c.pause();
// Real macrotask yield: React (and friends) schedule state updates on
// scheduler macrotasks, so an IO-gated animation ("start when in
// view") only arms after one. `await null` would only drain
// microtasks. MessageChannel, not setTimeout: hidden pages throttle
// DOM timers to ~1s, which would turn 30 yields into 30 real seconds.
const yieldReal = () => new Promise(r => {
  const ch = new MessageChannel();
  ch.port1.onmessage = () => r();
  ch.port2.postMessage(0);
});
c.step(TICK * 2);
await yieldReal(); // IO pump delivered; let gated starters arm
c.step(TICK * 3);
await yieldReal();

const movingSet = new Set();
for (const [el, x0, y0] of all) {
  const r = el.getBoundingClientRect();
  if (Math.abs(r.x - x0) > 0.5 || Math.abs(r.y - y0) > 0.5) movingSet.add(el);
}
// Movement is inherited: when a container moves, every descendant
// "moves". Track only topmost movers so the marquee is one track,
// not 37.
const topmost = [];
for (const el of movingSet) {
  let p = el.parentElement, top = true;
  while (p) { if (movingSet.has(p)) { top = false; break; } p = p.parentElement; }
  if (top) topmost.push(el);
}
// Style-mutated elements that have not moved yet may be one-shot
// movers about to fire; observe them too, movers first.
for (const el of mutated) {
  if (topmost.length >= MAX_TRACKS) break;
  if (!el.getBoundingClientRect || movingSet.has(el)) continue;
  let p = el.parentElement, covered = false;
  while (p) { if (movingSet.has(p)) { covered = true; break; } p = p.parentElement; }
  if (!covered) topmost.push(el);
}
mo.disconnect();

const tracks = topmost.slice(0, MAX_TRACKS).map(el => {
  const st = getComputedStyle(el);
  return {
    el,
    target: sel(el),
    property: st.transform && st.transform !== 'none' ? 'transform' : 'layout',
    scroll_w: el.scrollWidth, scroll_h: el.scrollHeight,
    xs: [], ys: [],
  };
});

// -- dense window: one sample per virtual frame ------------------------
const frames = Math.max(2, Math.min(Math.round(OBS_MS / TICK), 600));
for (let i = 0; i < frames; i++) {
  for (const t of tracks) {
    const r = t.el.getBoundingClientRect();
    t.xs.push(Math.round(r.x * 100) / 100);
    t.ys.push(Math.round(r.y * 100) / 100);
  }
  c.step(TICK);
  if (i % 5 === 4) await yieldReal(); else await null;
}

// -- wrap hunt: fast-forward to catch loop periods ---------------------
// A marquee's loop period is often minutes; nobody observes that in
// real time. Under virtual time we can step coarse chunks and watch
// for the position to jump against its own linear prediction.
const CHUNK = 100; // virtual ms between hunt samples
const HUNT_REAL_BUDGET_MS = 20000;
const huntStart = c.native.now();
let hunted = [];
for (const t of tracks) {
  for (const axis of ['x', 'y']) {
    const s = axis === 'x' ? t.xs : t.ys;
    const n = s.length;
    if (n < 8) continue;
    const deltas = [];
    for (let i = 1; i < n; i++) deltas.push(s[i] - s[i - 1]);
    const nz = deltas.filter(d => Math.abs(d) > 0.01);
    if (!nz.length) continue;
    const pos = nz.filter(d => d > 0).length;
    const mono = Math.max(pos, nz.length - pos) / nz.length;
    const v = (s[n - 1] - s[0]) / ((n - 1) * TICK); // px per virtual ms
    // Monotonic, still moving at the end: a loop candidate.
    const tail = Math.abs(s[n - 1] - s[n - 2]);
    if (mono >= 0.9 && Math.abs(v) * 1000 > 1 && tail > 0.01) {
      const extent = Math.max(t.scroll_w, t.scroll_h);
      const capMs = Math.min(200000, Math.max(20000, 2.5 * extent / Math.abs(v)));
      hunted.push({ t, axis, v, last: s[n - 1], capMs, wrap1: null, done: false });
      break; // one hunt axis per track
    }
  }
}
let vtime = 0;
while (hunted.some(h => !h.done) && c.native.now() - huntStart < HUNT_REAL_BUDGET_MS) {
  if (!hunted.some(h => !h.done && vtime < h.capMs)) break;
  c.step(CHUNK);
  vtime += CHUNK;
  await yieldReal();
  for (const h of hunted) {
    if (h.done) continue;
    if (vtime >= h.capMs) { h.done = true; continue; }
    const r = h.t.el.getBoundingClientRect();
    const p = h.axis === 'x' ? r.x : r.y;
    const pred = h.last + h.v * CHUNK;
    if (Math.abs(p - pred) > Math.max(24, Math.abs(h.v * CHUNK) * 4)) {
      // One wrap is enough: the fit derives the period from
      // jump/velocity. Confirming with a second wrap would double
      // the hunt for marginal precision.
      h.wrap1 = { t_ms: vtime, jump: p - pred };
      h.done = true;
    }
    h.last = p;
  }
}
for (const h of hunted) {
  h.t.wrap = { axis: h.axis, wrap1: h.wrap1 };
}
c.resume();
return {
  dt_ms: TICK,
  frames,
  window_ms: Math.round(frames * TICK),
  hunted_ms: vtime,
  tracks: tracks.map(t => ({
    target: t.target, property: t.property,
    scroll_w: t.scroll_w, scroll_h: t.scroll_h,
    xs: t.xs, ys: t.ys, wrap: t.wrap || null,
  })),
};
"""#
