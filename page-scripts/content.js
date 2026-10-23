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
