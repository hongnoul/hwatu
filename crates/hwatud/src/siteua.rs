//! Per-site user-agent overrides.
//!
//! Some sites serve a categorically better interface to mobile
//! browsers. Instagram is the canonical case: with a desktop UA you
//! get the sidebar-heavy desktop site, while a mobile UA gets the
//! touch-first PWA that Meta ships in place of a native app on
//! Windows/ChromeOS — full-bleed reels, vertical feed, no desktop
//! chrome. In a tiling-WM setup where a browser window is sized like
//! a phone anyway, the mobile UI plus wheel snap-paging (see
//! `smoothwheel`) is the closest a web view gets to the native app.
//!
//! Mechanism: WebKitGTK 6 exposes no is-main-frame flag on navigation
//! policy decisions, and `LoadEvent::Started` fires only after the
//! main request is already on the wire (verified empirically), so the
//! one deterministic hook is the *response* policy decision, which has
//! `is_main_frame_main_resource()`. When the main document's response
//! arrives and the view's UA doesn't match what the destination host
//! wants, the daemon flips the per-view `WebKitSettings` user-agent,
//! ignores the response, and reloads. That costs one extra request —
//! but only when crossing a rule boundary (entering or leaving a
//! matched site); steady-state navigation is untouched. Subframe
//! navigations never trigger a switch, matching per-tab UA semantics
//! in mainstream browsers (an embedded instagram iframe on another
//! site must not flip the whole page's UA). Non-GET main-frame
//! navigations (form POSTs) update the UA for subsequent loads but
//! are never restarted: replaying a POST as a fresh GET load would
//! drop the body.
//!
//! The override string is an iPhone Safari UA: WebKitGTK genuinely is
//! Safari-kin, so the spoof is nearly honest and the least likely to
//! trip Meta's browser gating.
//!
//! Config:
//! - `HWATU_MOBILE_UA_SITES`: comma-separated host list (subdomains
//!   match automatically). Default: `instagram.com`. `0`/`off`/empty
//!   after trim disables the feature entirely.
//! - `HWATU_MOBILE_UA`: override the UA string itself.

use webkit6::prelude::WebViewExt;

use webkit6::prelude::PolicyDecisionExt;

/// iPhone Safari. WebKit-on-WebKit: the rendering engine the UA
/// claims is the engine actually rendering.
const DEFAULT_MOBILE_UA: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) \
     AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1";

const DEFAULT_SITES: &str = "instagram.com";

/// Hosts that get the mobile UA. Empty vec = feature disabled.
fn sites() -> Vec<String> {
    let raw = match std::env::var("HWATU_MOBILE_UA_SITES") {
        Ok(v) => v,
        Err(_) => DEFAULT_SITES.to_string(),
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "0" || trimmed.eq_ignore_ascii_case("off") {
        return Vec::new();
    }
    trimmed
        .split(',')
        .map(|s| s.trim().trim_start_matches("www.").to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

fn mobile_ua() -> String {
    std::env::var("HWATU_MOBILE_UA")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_MOBILE_UA.to_string())
}

/// Host of `uri`, lowercased, `www.`-stripped. Avoids a URL-parsing
/// dependency: scheme://[userinfo@]host[:port]/...
fn host_of(uri: &str) -> Option<String> {
    let rest = uri.split("://").nth(1)?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    // Strip :port, but leave IPv6 literals ([::1]:8080) alone; they
    // never match a site rule anyway.
    let host = if host.starts_with('[') {
        host
    } else {
        host.split(':').next()?
    };
    if host.is_empty() {
        return None;
    }
    Some(host.trim_start_matches("www.").to_ascii_lowercase())
}

/// True when `host` is `site` or a subdomain of it.
fn host_matches(host: &str, site: &str) -> bool {
    host == site || host.strip_suffix(site).is_some_and(|p| p.ends_with('.'))
}

fn wants_mobile(uri: &str, rules: &[String]) -> bool {
    let Some(host) = host_of(uri) else {
        return false;
    };
    rules.iter().any(|site| host_matches(&host, site))
}

/// Handle a Response policy decision. Returns `true` when the
/// decision was consumed (UA flipped + load restarted); `false` lets
/// the caller's default handling proceed.
pub fn handle_response_decision(
    view: &webkit6::WebView,
    decision: &webkit6::ResponsePolicyDecision,
) -> bool {
    let rules = sites();
    if rules.is_empty() || !decision.is_main_frame_main_resource() {
        return false;
    }
    let Some(uri) = decision.request().and_then(|r| r.uri()) else {
        return false;
    };
    let Some(settings) = WebViewExt::settings(view) else {
        return false;
    };

    let current = settings.user_agent();
    let ours = |ua: &str| ua == mobile_ua() || ua == DEFAULT_MOBILE_UA;
    let desired: Option<String> = if wants_mobile(&uri, &rules) {
        Some(mobile_ua())
    } else if current.as_deref().is_some_and(ours) {
        None // leave a matched site: back to the engine default
    } else {
        return false; // someone else's UA (or already default): hands off
    };
    if current.as_deref() == desired.as_deref() {
        return false; // already correct: steady state, zero cost
    }

    settings.set_user_agent(desired.as_deref());
    // Replaying a POST as a load_uri GET would drop the body; the
    // flipped UA simply applies from the next load on.
    let method = decision.request().and_then(|r| r.http_method());
    if method
        .as_deref()
        .is_some_and(|m| !m.eq_ignore_ascii_case("GET"))
    {
        return false;
    }
    decision.ignore();
    view.load_uri(&uri);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_extraction() {
        assert_eq!(
            host_of("https://instagram.com/reels/"),
            Some("instagram.com".into())
        );
        assert_eq!(
            host_of("https://www.instagram.com/"),
            Some("instagram.com".into())
        );
        assert_eq!(
            host_of("https://user@x.com:8443/p?q#f"),
            Some("x.com".into())
        );
        assert_eq!(host_of("about:blank"), None);
        assert_eq!(host_of("https:///nohost"), None);
    }

    #[test]
    fn suffix_matching_is_label_aware() {
        assert!(host_matches("instagram.com", "instagram.com"));
        assert!(host_matches("help.instagram.com", "instagram.com"));
        // Must not match lookalike registrable domains.
        assert!(!host_matches("notinstagram.com", "instagram.com"));
        assert!(!host_matches("instagram.com.evil.example", "instagram.com"));
    }

    #[test]
    fn rule_gating() {
        std::env::remove_var("HWATU_MOBILE_UA_SITES");
        assert!(wants_mobile("https://www.instagram.com/reels/", &sites()));
        assert!(!wants_mobile("https://example.com/", &sites()));

        std::env::set_var("HWATU_MOBILE_UA_SITES", "off");
        assert!(sites().is_empty());

        std::env::set_var("HWATU_MOBILE_UA_SITES", "instagram.com, m.example.org");
        let rules = sites();
        assert!(wants_mobile("https://m.example.org/x", &rules));
        assert!(wants_mobile("https://instagram.com/", &rules));
        assert!(!wants_mobile("https://example.org/", &rules));
        std::env::remove_var("HWATU_MOBILE_UA_SITES");
    }

    #[test]
    fn ua_override_env() {
        std::env::remove_var("HWATU_MOBILE_UA");
        assert_eq!(mobile_ua(), DEFAULT_MOBILE_UA);
        std::env::set_var("HWATU_MOBILE_UA", "TestUA/1.0");
        assert_eq!(mobile_ua(), "TestUA/1.0");
        std::env::remove_var("HWATU_MOBILE_UA");
    }
}
