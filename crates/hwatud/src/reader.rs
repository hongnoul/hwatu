// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! Reader mode (roadmap H34): extract the page's main article and
//! re-render it with clean typography. Safari's signature feature,
//! readability-lite: no dependency, a scoring pass over text blocks.
//!
//! Toggle (default ctrl+shift+r is taken by hard reload, so `alt+r`):
//! on enter, the extractor picks the highest-scoring container
//! (paragraph text mass, link-density penalty), clones it into a
//! fixed overlay with reading CSS (user fonts, measure-width column,
//! dark-mode aware), and freezes page scroll. Esc or the toggle
//! exits back to the live page untouched — the overlay is additive,
//! nothing in the original DOM is destroyed.

use webkit6::prelude::*;

pub const READER_JS: &str = r#"(() => {
  'use strict';
  if (window.__hwatuReader) return;

  const OVERLAY_ID = '__hwatu_reader__';

  const linkDensity = (el) => {
    const text = el.innerText || '';
    if (!text.length) return 1;
    let linked = 0;
    for (const a of el.querySelectorAll('a')) linked += (a.innerText || '').length;
    return linked / text.length;
  };

  const pick = () => {
    // Prefer semantic containers, fall back to scored divs.
    const semantic = document.querySelector('article, [role="article"], main');
    if (semantic && (semantic.innerText || '').length > 500 && linkDensity(semantic) < 0.5) {
      return semantic;
    }
    let best = null, bestScore = 0;
    for (const el of document.querySelectorAll('div, section, td')) {
      const paras = el.querySelectorAll('p');
      if (paras.length < 3) continue;
      let mass = 0;
      for (const p of paras) mass += (p.innerText || '').length;
      const score = mass * (1 - linkDensity(el));
      if (score > bestScore) { bestScore = score; best = el; }
    }
    return bestScore > 500 ? best : null;
  };

  const enter = () => {
    if (document.getElementById(OVERLAY_ID)) return 'reader already';
    const src = pick();
    if (!src) return 'no article found';
    const overlay = document.createElement('div');
    overlay.id = OVERLAY_ID;
    overlay.style.cssText =
      'position:fixed;inset:0;z-index:2147483646;overflow-y:auto;' +
      'background:#faf8f3;color:#222;';
    if (matchMedia('(prefers-color-scheme: dark)').matches) {
      overlay.style.background = '#1b1b1f';
      overlay.style.color = '#d8d8d2';
    }
    const column = document.createElement('div');
    column.style.cssText =
      'max-width:42rem;margin:0 auto;padding:3rem 1.5rem 6rem;' +
      'font: 1.125rem/1.7 Georgia, "Noto Serif", serif;';
    const title = document.createElement('h1');
    title.textContent = document.title;
    title.style.cssText = 'font-size:1.6rem;line-height:1.3;margin-bottom:1.5rem;';
    column.appendChild(title);
    const body = src.cloneNode(true);
    // Strip the noise that survives cloning.
    for (const sel of ['script', 'style', 'iframe', 'nav', 'aside', 'form',
                       '[role="navigation"]', '[class*="share"]', '[class*="related"]']) {
      for (const el of body.querySelectorAll(sel)) el.remove();
    }
    for (const el of body.querySelectorAll('*')) {
      el.removeAttribute('style');
      el.removeAttribute('class');
    }
    for (const img of body.querySelectorAll('img')) {
      img.style.maxWidth = '100%';
      img.style.height = 'auto';
    }
    column.appendChild(body);
    overlay.appendChild(column);
    document.documentElement.appendChild(overlay);
    document.documentElement.style.overflow = 'hidden';
    const onKey = (e) => { if (e.key === 'Escape') exit(); };
    window.addEventListener('keydown', onKey, true);
    overlay.__hwatuOnKey = onKey;
    return 'reader on';
  };

  const exit = () => {
    const overlay = document.getElementById(OVERLAY_ID);
    if (!overlay) return 'reader already off';
    if (overlay.__hwatuOnKey) window.removeEventListener('keydown', overlay.__hwatuOnKey, true);
    overlay.remove();
    document.documentElement.style.overflow = '';
    return 'reader off';
  };

  const toggle = () => document.getElementById(OVERLAY_ID) ? exit() : enter();

  window.__hwatuReader = { enter, exit, toggle };
})();"#;

/// Inject the reader machinery into a WebView, same contract as the
/// other `wire_view`s.
pub fn wire_view(view: &webkit6::WebView) {
    let Some(ucm) = view.user_content_manager() else {
        return;
    };
    let script = webkit6::UserScript::new(
        READER_JS,
        webkit6::UserContentInjectedFrames::TopFrame,
        webkit6::UserScriptInjectionTime::Start,
        &[],
        &[],
    );
    ucm.add_script(&script);
}

/// JS expression toggling reader mode.
pub fn toggle_js() -> &'static str {
    "window.__hwatuReader && __hwatuReader.toggle()"
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The overlay must be additive (original DOM untouched: clone,
    /// never move) and exit must restore scroll.
    #[test]
    fn overlay_is_additive_and_reversible() {
        assert!(READER_JS.contains("cloneNode(true)"));
        assert!(READER_JS.contains("overflow = ''"));
        assert!(READER_JS.contains("overlay.remove()"));
    }

    /// Fails open on pages with no article-shaped content.
    #[test]
    fn fails_open_without_article() {
        assert!(READER_JS.contains("'no article found'"));
        assert!(READER_JS.contains("bestScore > 500"));
    }

    /// Escape exits; scripts/styles/navigation stripped from the clone.
    #[test]
    fn escape_exits_and_noise_stripped() {
        assert!(READER_JS.contains("'Escape'"));
        assert!(READER_JS.contains("'script', 'style', 'iframe', 'nav'"));
    }

    /// Link-density penalty keeps comment/nav-heavy containers out.
    #[test]
    fn link_density_penalty_present() {
        assert!(READER_JS.contains("linkDensity"));
        assert!(READER_JS.contains("1 - linkDensity(el)"));
    }

    #[test]
    fn toggle_js_shape() {
        assert!(toggle_js().contains("__hwatuReader.toggle()"));
    }
}
