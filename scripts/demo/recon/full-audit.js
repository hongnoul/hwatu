(() => {
  const R = el => { const r = el.getBoundingClientRect();
    return [Math.round(r.left), Math.round(r.top+scrollY), Math.round(r.width), Math.round(r.height)]; };
  const pick = (el, props) => { const s = getComputedStyle(el); const o = {};
    props.forEach(p => o[p] = s.getPropertyValue(p)); return o; };
  const T = ['font-family','font-size','font-weight','line-height','letter-spacing','color'];
  const out = {viewport:[innerWidth,innerHeight,devicePixelRatio], doc:document.documentElement.scrollHeight};

  // 1. section map
  out.sections = [document.querySelector('header'), ...document.querySelectorAll('main > *'), document.querySelector('footer')]
    .filter(e=>e && e.getBoundingClientRect().height>0)
    .map((el,i)=>({i, tag:el.tagName.toLowerCase(), cls:(el.className+'').trim().slice(0,110), rect:R(el),
      heading:(el.querySelector('h1,h2,h3')||{}).textContent?.trim().slice(0,60)||null,
      bg:getComputedStyle(el).backgroundColor}));

  // 2. type roles
  const roleSel = {
    h1:'h1', h1Copy:'.hero-section__title-copy', h2:'h2', h3:'h3',
    heroEyebrow:'.hero-section__eyebrow', navLink:'header nav li a[href]',
    ctaPrimary:'.hero-section-container a[href*=register], a.hds-button--primary, .hds-button',
    bodyP:'.section-column-layout p, main section:nth-of-type(2) p',
    statValue:'#stat-payment-methods-value', statDesc:'#stat-payment-methods-description',
    footerLink:'footer a[href]', footerHead:'footer [class*=column] > *:first-child',
    pillarTitle:'.pillar__title, [class*=pillar] h3', tabLabel:'[role=tab]',
  };
  out.type = {};
  for (const [k,sel] of Object.entries(roleSel)) { const el=document.querySelector(sel);
    if (el) out.type[k] = {sel, sample:el.textContent.trim().slice(0,36), rect:R(el), ...pick(el,T)}; }

  // 3. css custom props (design tokens) from :root
  const rs = getComputedStyle(document.documentElement);
  const tokens = {};
  for (const sheet of document.styleSheets) {
    let rules; try { rules = sheet.cssRules; } catch(e){ continue; }
    for (const rule of rules) {
      if (rule.style && (rule.selectorText===':root' || /hds-mode|hds-color-mode/.test(rule.selectorText||''))) {
        for (const p of rule.style) if (p.startsWith('--')) tokens[p] = rule.style.getPropertyValue(p).trim();
      }
    }
  }
  out.tokenCount = Object.keys(tokens).length;
  out.tokens = tokens;

  // 4. buttons / radii / shadows
  const btn = document.querySelector('.hero-section-container a[class*=button], .hds-button');
  if (btn) out.button = {rect:R(btn), ...pick(btn,[...T,'background-color','background-image','border-radius','padding','box-shadow','border'])};
  const card = document.querySelector('[class*=card]');
  if (card) out.card = {cls:String(card.className).slice(0,60), ...pick(card,['border-radius','box-shadow','background-color','padding'])};

  return JSON.stringify(out);
})()
