// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! Forced dark mode (roadmap H15): a prefers-color-scheme override
//! plus an injected-CSS darkener, per-site toggleable, persisted on
//! the site store.
//!
//! Two independent tiers:
//!
//! 1. **Scheme hint** — pages that support dark mode natively get it
//!    for free: hwatud follows the desktop's color-scheme (GTK's
//!    `gtk-application-prefer-dark-theme` / the `org.freedesktop.
//!    appearance` portal value surfaces through the GTK setting) so
//!    `prefers-color-scheme: dark` resolves dark. That much is
//!    engine-native and always safe.
//!
//! 2. **Forced darkener** — for pages with no dark stylesheet, a
//!    `dark_mode` action (default ctrl+shift+d) injects a CSS
//!    invert+hue-rotate filter (media un-inverted), the classic
//!    darkreader-lite technique that needs no per-element analysis.
//!    The toggle persists per host on the site store, so a site you
//!    darkened stays dark and a site that double-inverts can be
//!    excluded once and stays excluded.
//!
//! `"dark_mode": true` in config.json makes the forced darkener the
//! default for every site; per-host toggles then record exceptions.

/// CSS for the forced darkener. html gets inverted, media inverted
/// back. The dark background on :root keeps white flashes down.
pub const DARK_CSS: &str = "\
html { filter: invert(1) hue-rotate(180deg) !important; background: #121212 !important; }\n\
img, video, canvas, picture, svg image, [style*='background-image'] {\n\
  filter: invert(1) hue-rotate(180deg) !important;\n\
}";

/// JS that applies or removes the darkener in a live page.
pub fn apply_js(on: bool) -> String {
    if on {
        format!(
            r#"(() => {{
  if (document.getElementById('__hwatu_dark__')) return 'dark already';
  const s = document.createElement('style');
  s.id = '__hwatu_dark__';
  s.textContent = {css};
  (document.head || document.documentElement).appendChild(s);
  return 'dark on';
}})()"#,
            css = serde_json::to_string(DARK_CSS).unwrap_or_default()
        )
    } else {
        r#"(() => {
  const s = document.getElementById('__hwatu_dark__');
  if (!s) return 'dark already off';
  s.remove();
  return 'dark off';
})()"#
            .to_string()
    }
}

/// Whether the forced darkener should be on for `host`, combining the
/// global default with the per-site exception list.
pub fn should_darken(store: &crate::sitedata::SiteStore, host: &str) -> bool {
    let default_on = crate::window::config_value("dark_mode")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    match store.dark_mode(host) {
        Some(per_site) => per_site,
        None => default_on,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_js_round_trip_markers() {
        let on = apply_js(true);
        assert!(on.contains("__hwatu_dark__"));
        assert!(on.contains("invert(1)"));
        // CSS is JSON-escaped into the script, not spliced raw.
        assert!(on.contains("\\n"));
        let off = apply_js(false);
        assert!(off.contains("s.remove()"));
    }

    #[test]
    fn media_uninverted() {
        // Double inversion restores photos/video to natural colors.
        assert!(DARK_CSS.contains("img, video, canvas"));
        let occurrences = DARK_CSS.matches("invert(1)").count();
        assert_eq!(occurrences, 2, "html inverted, media inverted back");
    }

    #[test]
    fn per_site_beats_global_default() {
        let store = crate::sitedata::SiteStore::ephemeral();
        // No config file in tests: global default is off.
        assert!(!should_darken(&store, "example.com"));
        store.set_dark_mode("example.com", true);
        assert!(should_darken(&store, "example.com"));
        store.set_dark_mode("example.com", false);
        assert!(!should_darken(&store, "example.com"));
    }
}
