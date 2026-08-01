//! Neutralize CPU-melting decorative blurs behind video ("ambient glow").
//!
//! Short-form video pages (YouTube Shorts, Instagram Reels, TikTok)
//! layer a stretched copy of the video (a `<canvas>` or a second
//! `<video>`) behind the player with `filter: blur(40px)` to fake a
//! phone-style ambient glow. Phones composite that blur on the GPU;
//! WebKitGTK's Skia backend rasterizes large `filter: blur()` on
//! animated content on the CPU, every frame. Measured on YouTube
//! Shorts (952x1222 glow layer, blur(40px), WebKitGTK 2.52): the web
//! process main thread pegs at ~100% inside SkBlurImageFilter /
//! FEDropShadowSkiaApplier and the whole page — video presentation
//! included — collapses from 100+ rAF fps to ~7-34. Hiding that one
//! element restores full frame rate while the video itself keeps
//! decoding at its native 30fps (0 dropped) throughout.
//!
//! The shield scans every 1.5s (plus once at load): for each
//! `<video>`/`<canvas>` it checks the element and up to 4 ancestors
//! for a computed `filter` with `blur(>= 16px)` covering a large area
//! (>= 200k CSS px²), and hides the match. The scan is cheap — pages
//! have few media elements — and re-applies if the site rewrites the
//! style attribute. Small blurs (glassmorphism, drop shadows on
//! icons) and `backdrop-filter` are untouched.
//!
//! Trade-off: the ambient glow disappears (dark backdrop instead).
//! Full-rate playback beats a decorative halo; anyone who prefers the
//! glow can turn the shield off.
//!
//! Gate: `HWATU_BLUR_SHIELD=0|off|false` disables the shield.

const BLUR_SHIELD_JS: &str = r#"(() => {
  'use strict';

  const MIN_RADIUS_PX = 16;
  const MIN_AREA_PX2 = 200000;
  const MAX_ANCESTORS = 4;

  const blurRadius = (el) => {
    let filter;
    try { filter = getComputedStyle(el).filter; } catch (e) { return 0; }
    if (!filter || filter === 'none') return 0;
    const m = filter.match(/blur\((\d+(?:\.\d+)?)px\)/);
    return m ? parseFloat(m[1]) : 0;
  };

  const neutralize = (el) => {
    // visibility (not filter:none): an unblurred stretched glow layer
    // looks broken; hidden just leaves the page's dark backdrop.
    el.style.setProperty('visibility', 'hidden', 'important');
    el.style.setProperty('filter', 'none', 'important');
  };

  const scan = () => {
    let media;
    try { media = document.querySelectorAll('video, canvas'); } catch (e) { return; }
    for (const m of media) {
      let el = m;
      for (let hop = 0; el && hop <= MAX_ANCESTORS; hop++, el = el.parentElement) {
        if (blurRadius(el) < MIN_RADIUS_PX) continue;
        const rect = el.getBoundingClientRect();
        if (rect.width * rect.height < MIN_AREA_PX2) continue;
        neutralize(el);
      }
    }
  };

  // Media elements appear late on SPA feeds; a light interval catches
  // them (and re-neutralizes if the site rewrites the style attribute).
  setInterval(scan, 1500);
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', scan, { once: true });
  } else {
    scan();
  }
})();"#;

/// True unless `HWATU_BLUR_SHIELD` explicitly disables the shield.
fn enabled() -> bool {
    !matches!(
        std::env::var("HWATU_BLUR_SHIELD").as_deref(),
        Ok("0") | Ok("off") | Ok("false")
    )
}

/// Inject the blur shield into a WebView. Must run on every view
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
        BLUR_SHIELD_JS,
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
        std::env::remove_var("HWATU_BLUR_SHIELD");
        assert!(enabled());
        std::env::set_var("HWATU_BLUR_SHIELD", "0");
        assert!(!enabled());
        std::env::set_var("HWATU_BLUR_SHIELD", "off");
        assert!(!enabled());
        std::env::set_var("HWATU_BLUR_SHIELD", "1");
        assert!(enabled());
        std::env::remove_var("HWATU_BLUR_SHIELD");
    }

    /// The shield must only fire on big blurs over large areas around
    /// media elements — the thresholds are the safety mechanism that
    /// keeps glassmorphism UI and icon shadows alive.
    #[test]
    fn script_has_conservative_thresholds() {
        assert!(BLUR_SHIELD_JS.contains("MIN_RADIUS_PX = 16"));
        assert!(BLUR_SHIELD_JS.contains("MIN_AREA_PX2 = 200000"));
        assert!(BLUR_SHIELD_JS.contains("'video, canvas'"));
    }

    /// Neutralizing must hide the layer, not just strip the filter: an
    /// unblurred stretched glow canvas looks broken.
    #[test]
    fn script_hides_rather_than_unblurs() {
        assert!(BLUR_SHIELD_JS.contains("'visibility', 'hidden', 'important'"));
    }

    /// The scan must walk ancestors (the blur usually sits on a
    /// wrapper div, not the canvas itself) and re-run periodically for
    /// SPA feeds.
    #[test]
    fn script_walks_ancestors_and_rescans() {
        assert!(BLUR_SHIELD_JS.contains("MAX_ANCESTORS"));
        assert!(BLUR_SHIELD_JS.contains("parentElement"));
        assert!(BLUR_SHIELD_JS.contains("setInterval(scan, 1500)"));
    }

    /// Style reads must fail open: a detached element or hostile page
    /// where getComputedStyle throws keeps its native behavior.
    #[test]
    fn script_fails_open() {
        assert!(BLUR_SHIELD_JS.contains("catch (e) { return 0; }"));
    }
}
