(() => {
  const R = el => { const r = el.getBoundingClientRect();
    return [Math.round(r.left), Math.round(r.top+scrollY), Math.round(r.width), Math.round(r.height)]; };
  const CS = (el, props) => { const s = getComputedStyle(el); const o={};
    props.forEach(p=>o[p]=s.getPropertyValue(p)); return o; };
  const TYPE = ['font-family','font-size','font-weight','line-height','letter-spacing','color','text-transform'];
  const BOX = ['padding','margin','background-color','background-image','border-radius','box-shadow','border','gap','display','max-width'];
  const out = {viewport:[innerWidth,innerHeight,devicePixelRatio], doc: document.documentElement.scrollHeight, sections: [], type: {}, chrome: {}};

  // section map
  const secs = [document.querySelector('header'), ...document.querySelectorAll('main > *'), document.querySelector('footer')].filter(Boolean);
  secs.forEach((el,i)=>{
    const heading = el.querySelector('h1,h2,h3');
    out.sections.push({i, tag: el.tagName.toLowerCase(), cls: (el.className+'').trim().slice(0,100),
      rect: R(el), heading: heading? heading.textContent.trim().slice(0,60): null,
      bg: getComputedStyle(el).backgroundColor});
  });

  // type samples: one per role
  const roles = {
    h1: 'h1', h2: 'h2', h3: 'h3',
    navLink: 'header nav a',
    body: 'main p',
    cta: 'main a[class*="Button"], main a[class*="button"], .hero-section-container a',
    footerLink: 'footer a', footerHeading: 'footer h2, footer h3, footer [class*=heading]',
    eyebrow: '[class*=eyebrow], [class*=Eyebrow]',
    code: 'code, pre'
  };
  for (const [k, sel] of Object.entries(roles)) {
    const el = document.querySelector(sel);
    if (el) out.type[k] = {sel, text: el.textContent.trim().slice(0,40), ...CS(el, TYPE)};
  }
  return JSON.stringify(out);
})()
