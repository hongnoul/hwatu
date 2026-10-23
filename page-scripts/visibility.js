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
