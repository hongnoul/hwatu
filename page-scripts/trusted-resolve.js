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
