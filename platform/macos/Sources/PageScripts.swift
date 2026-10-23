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
  if (!handled && el.form) el.form.requestSubmit ? el.form.requestSubmit() : el.form.submit();
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
