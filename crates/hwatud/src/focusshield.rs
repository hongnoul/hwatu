//! Keep pages playing when the window loses focus.
//!
//! Sites (Instagram Reels, YouTube, TikTok) pause media the moment the
//! page reports hidden or the window blurs. hwatu treats losing WM
//! focus as a non-event for page content: an injected user script pins
//! the Page Visibility API to "visible", makes `document.hasFocus()`
//! return true, and swallows window/document-level visibility and
//! focus-loss events before page listeners can see them. Element-level
//! focus management (inputs, editors) is untouched.
//!
//! Gate: `HWATU_FOCUS_SHIELD=0|off|false` disables the shield.

const FOCUS_SHIELD_JS: &str = r#"(() => {
  'use strict';

  const define = (obj, prop, getter) => {
    try {
      Object.defineProperty(obj, prop, { get: getter, configurable: true });
    } catch (e) { /* fail open: leave the native property alone */ }
  };

  // Page Visibility API: always visible.
  define(Document.prototype, 'hidden', () => false);
  define(Document.prototype, 'visibilityState', () => 'visible');
  define(Document.prototype, 'webkitHidden', () => false);
  define(Document.prototype, 'webkitVisibilityState', () => 'visible');

  // Focus probes: the page always believes it holds focus.
  try { Document.prototype.hasFocus = function () { return true; }; } catch (e) {}

  // Swallow window/document-targeted visibility and focus-loss events
  // before page listeners run. This script is injected at document
  // start, so these capture listeners register ahead of any page
  // listener and stopImmediatePropagation() silences them all.
  // Element-targeted focus/blur (inputs, editors) passes through: the
  // target check skips anything that is not the window or document.
  const swallow = (e) => {
    if (e.target === window || e.target === document) {
      e.stopImmediatePropagation();
    }
  };
  for (const type of ['visibilitychange', 'webkitvisibilitychange']) {
    window.addEventListener(type, swallow, true);
    document.addEventListener(type, swallow, true);
  }
  for (const type of ['blur', 'focus', 'focusin', 'focusout']) {
    window.addEventListener(type, swallow, true);
  }
})();"#;

/// True unless `HWATU_FOCUS_SHIELD` explicitly disables the shield.
fn enabled() -> bool {
    !matches!(
        std::env::var("HWATU_FOCUS_SHIELD").as_deref(),
        Ok("0") | Ok("off") | Ok("false")
    )
}

/// Inject the focus shield into a WebView. Must run on every view
/// (prewarm pool and popups) before page content loads, same contract
/// as `console::wire_view`.
pub fn wire_view(view: &webkit6::WebView) {
    use webkit6::prelude::*;
    if !enabled() {
        return;
    }
    let Some(ucm) = view.user_content_manager() else {
        return;
    };
    let script = webkit6::UserScript::new(
        FOCUS_SHIELD_JS,
        webkit6::UserContentInjectedFrames::AllFrames,
        webkit6::UserScriptInjectionTime::Start,
        &[],
        &[],
    );
    ucm.add_script(&script);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The env gate must treat only explicit "0"/"off"/"false" as off;
    /// absence and anything else keep the shield on.
    #[test]
    fn env_gate_semantics() {
        std::env::remove_var("HWATU_FOCUS_SHIELD");
        assert!(enabled());
        std::env::set_var("HWATU_FOCUS_SHIELD", "0");
        assert!(!enabled());
        std::env::set_var("HWATU_FOCUS_SHIELD", "off");
        assert!(!enabled());
        std::env::set_var("HWATU_FOCUS_SHIELD", "1");
        assert!(enabled());
        std::env::remove_var("HWATU_FOCUS_SHIELD");
    }

    /// The shield must pin the complete visibility surface, including
    /// the webkit-prefixed legacy properties Instagram still probes.
    #[test]
    fn script_pins_visibility_surface() {
        for prop in [
            "'hidden'",
            "'visibilityState'",
            "'webkitHidden'",
            "'webkitVisibilityState'",
        ] {
            assert!(FOCUS_SHIELD_JS.contains(prop), "missing {prop}");
        }
        assert!(FOCUS_SHIELD_JS.contains("'visible'"));
        assert!(FOCUS_SHIELD_JS.contains("hasFocus"));
    }

    /// Event suppression must be scoped: stopImmediatePropagation only
    /// fires for window/document-targeted events, so element-level
    /// focus management keeps working.
    #[test]
    fn script_scopes_event_suppression() {
        assert!(FOCUS_SHIELD_JS.contains("stopImmediatePropagation"));
        assert!(
            FOCUS_SHIELD_JS.contains("e.target === window || e.target === document"),
            "swallow must check the event target"
        );
        for ev in ["visibilitychange", "'blur'", "'focus'", "'focusin'", "'focusout'"] {
            assert!(FOCUS_SHIELD_JS.contains(ev), "missing event {ev}");
        }
    }

    /// Property overrides must fail open: a page (or engine) where
    /// defineProperty throws keeps its native behavior.
    #[test]
    fn script_fails_open() {
        assert!(FOCUS_SHIELD_JS.contains("try {"));
        assert!(FOCUS_SHIELD_JS.contains("configurable: true"));
    }
}
