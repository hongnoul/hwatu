// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! Link hints (roadmap H10): keyboard navigation to links, qutebrowser
//! `f` style. Press the hint key, every visible interactable gets a
//! short home-row label, type the label, the element activates.
//!
//! Variants: `follow` clicks in place, `newwin` opens links as a new
//! toplevel (window.open -> the popup path -> a real window, WM-tiled
//! like any other), `yank` copies the target href to the clipboard
//! (posted back to the daemon over a script message so GDK owns the
//! clipboard; pages' navigator.clipboard needs permissions we don't
//! want to grant hint mode).
//!
//! Everything after the trigger key lives in page-side JS: hint mode
//! installs one capture-phase keydown listener that consumes keys
//! until a label completes, Escape/no-match exits, scroll/blur
//! dismisses. Unknown pages fail open — no interactables, no overlay.

use webkit6::prelude::*;

/// Script-message handler name for yank results.
pub const YANK_HANDLER: &str = "hwatuHintYank";

const HINTS_JS: &str = r#"(() => {
  'use strict';
  if (window.__hwatuHints) return;

  const ALPHABET = 'asdfghjkl';
  const state = { active: false, mode: 'follow', hints: [], typed: '', root: null, onKey: null };

  const labels = (n) => {
    // Shortest uniform-length labels over the home row: 1 char covers
    // 9, 2 chars 81, 3 chars 729 — enough for any real page.
    let len = 1;
    while (Math.pow(ALPHABET.length, len) < n) len++;
    const out = [];
    for (let i = 0; i < n; i++) {
      let s = '', x = i;
      for (let j = 0; j < len; j++) { s = ALPHABET[x % ALPHABET.length] + s; x = Math.floor(x / ALPHABET.length); }
      out.push(s);
    }
    return out;
  };

  const candidates = () => {
    const sel = 'a[href], button, input, select, textarea, summary, ' +
      '[onclick], [role="link"], [role="button"], [contenteditable="true"]';
    let els;
    try { els = document.querySelectorAll(sel); } catch (e) { return []; }
    const seen = new Set();
    const out = [];
    for (const el of els) {
      if (seen.has(el)) continue;
      seen.add(el);
      const r = el.getBoundingClientRect();
      if (r.width < 2 || r.height < 2) continue;
      if (r.bottom < 0 || r.right < 0 || r.top > innerHeight || r.left > innerWidth) continue;
      const style = getComputedStyle(el);
      if (style.visibility === 'hidden' || style.display === 'none' || style.opacity === '0') continue;
      // Center must resolve to the element or its subtree (not covered).
      const cx = Math.max(0, Math.min(r.left + r.width / 2, innerWidth - 1));
      const cy = Math.max(0, Math.min(r.top + r.height / 2, innerHeight - 1));
      const at = document.elementFromPoint(cx, cy);
      if (!at || (at !== el && !el.contains(at) && !at.contains(el))) continue;
      out.push({ el, rect: r });
    }
    return out;
  };

  const dismiss = () => {
    if (!state.active) return;
    state.active = false;
    state.typed = '';
    if (state.onKey) { window.removeEventListener('keydown', state.onKey, true); state.onKey = null; }
    window.removeEventListener('scroll', dismiss, true);
    if (state.root) { state.root.remove(); state.root = null; }
    state.hints = [];
  };

  const activate = (el) => {
    const mode = state.mode;
    dismiss();
    if (mode === 'yank') {
      const href = el.href || el.getAttribute('href') || '';
      if (href && window.webkit && webkit.messageHandlers && webkit.messageHandlers.hwatuHintYank) {
        webkit.messageHandlers.hwatuHintYank.postMessage(String(href));
      }
      return;
    }
    if (mode === 'newwin') {
      const href = el.href || el.getAttribute('href');
      if (href) { window.open(href, '_blank'); return; }
      // No href: fall through to a plain activation.
    }
    if (el.matches('input, select, textarea, [contenteditable="true"]') &&
        !el.matches('input[type=button], input[type=submit], input[type=checkbox], input[type=radio]')) {
      el.focus();
      return;
    }
    el.focus({ preventScroll: true });
    el.click();
  };

  const redraw = () => {
    for (const h of state.hints) {
      if (state.typed && !h.label.startsWith(state.typed)) {
        h.tag.style.display = 'none';
      } else {
        h.tag.style.display = '';
        h.tag.innerHTML = '<b>' + h.label.slice(0, state.typed.length) + '</b>' + h.label.slice(state.typed.length);
      }
    }
  };

  const start = (mode) => {
    dismiss();
    const cands = candidates();
    if (!cands.length) return 'no hints';
    state.mode = mode || 'follow';
    state.active = true;
    const root = document.createElement('div');
    root.id = '__hwatu_hints__';
    root.style.cssText = 'position:fixed;inset:0;z-index:2147483647;pointer-events:none;';
    const labs = labels(cands.length);
    state.hints = cands.map((c, i) => {
      const tag = document.createElement('span');
      tag.textContent = labs[i];
      tag.style.cssText =
        'position:absolute;left:' + Math.max(0, c.rect.left) + 'px;top:' + Math.max(0, c.rect.top) + 'px;' +
        'background:#1a1a1a;color:#ffd75f;font:bold 12px monospace;padding:1px 4px;' +
        'border-radius:3px;border:1px solid #444;box-shadow:0 1px 4px rgba(0,0,0,.5);';
      root.appendChild(tag);
      return { el: c.el, label: labs[i], tag };
    });
    (document.body || document.documentElement).appendChild(root);
    state.root = root;
    state.onKey = (e) => {
      if (e.ctrlKey || e.altKey || e.metaKey) { dismiss(); return; }
      e.preventDefault();
      e.stopImmediatePropagation();
      if (e.key === 'Escape') { dismiss(); return; }
      if (e.key === 'Backspace') { state.typed = state.typed.slice(0, -1); redraw(); return; }
      const ch = e.key.length === 1 ? e.key.toLowerCase() : '';
      if (!ALPHABET.includes(ch)) return;
      state.typed += ch;
      const live = state.hints.filter((h) => h.label.startsWith(state.typed));
      if (!live.length) { dismiss(); return; }
      if (live.length === 1 && live[0].label === state.typed) { activate(live[0].el); return; }
      redraw();
    };
    window.addEventListener('keydown', state.onKey, true);
    window.addEventListener('scroll', dismiss, true);
    return cands.length + ' hints';
  };

  window.__hwatuHints = { start, dismiss, active: () => state.active };
})();"#;

/// Inject the hint machinery into a WebView (prewarm pool and popups),
/// same contract as the other `wire_view`s. `on_yank` receives hrefs
/// from yank-mode activations.
pub fn wire_view(view: &webkit6::WebView, on_yank: impl Fn(String) + 'static) {
    let Some(ucm) = view.user_content_manager() else {
        return;
    };
    let script = webkit6::UserScript::new(
        HINTS_JS,
        // Main frame only: hint labels in cross-origin iframes would
        // render at wrong coordinates and can't be activated anyway.
        webkit6::UserContentInjectedFrames::TopFrame,
        webkit6::UserScriptInjectionTime::Start,
        &[],
        &[],
    );
    ucm.add_script(&script);
    ucm.register_script_message_handler(YANK_HANDLER, None);
    ucm.connect_script_message_received(Some(YANK_HANDLER), move |_, value| {
        let href = value.to_str().to_string();
        if !href.is_empty() {
            on_yank(href);
        }
    });
}

/// JS expression that enters hint mode. `mode`: follow | newwin | yank.
pub fn start_js(mode: &str) -> String {
    format!("window.__hwatuHints && __hwatuHints.start('{mode}')")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hint keys must be consumed in capture phase and Escape must
    /// exit: half-typed labels leaking to the page as keystrokes is
    /// the classic hint-mode bug.
    #[test]
    fn script_consumes_keys_and_escapes() {
        assert!(HINTS_JS.contains("e.stopImmediatePropagation()"));
        assert!(HINTS_JS.contains("e.preventDefault()"));
        assert!(HINTS_JS.contains("'Escape'"));
        assert!(HINTS_JS.contains("window.addEventListener('keydown', state.onKey, true)"));
    }

    /// Fails open: zero candidates -> no overlay, page untouched.
    #[test]
    fn script_fails_open_without_candidates() {
        assert!(HINTS_JS.contains("if (!cands.length) return 'no hints'"));
    }

    /// Yank posts through the registered handler name, never through
    /// navigator.clipboard (which would need permission grants).
    #[test]
    fn yank_uses_script_message() {
        assert!(HINTS_JS.contains("webkit.messageHandlers.hwatuHintYank"));
        assert!(!HINTS_JS.contains("navigator.clipboard"));
        assert_eq!(YANK_HANDLER, "hwatuHintYank");
    }

    /// Covered elements are filtered via elementFromPoint, and
    /// off-screen ones by viewport intersection.
    #[test]
    fn candidates_are_visible_and_uncovered() {
        assert!(HINTS_JS.contains("elementFromPoint"));
        assert!(HINTS_JS.contains("r.top > innerHeight"));
    }

    #[test]
    fn start_js_shapes() {
        assert_eq!(
            start_js("follow"),
            "window.__hwatuHints && __hwatuHints.start('follow')"
        );
    }
}
