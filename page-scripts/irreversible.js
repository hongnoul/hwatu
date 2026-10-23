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
