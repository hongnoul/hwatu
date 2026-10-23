// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Justin Hong
//! Structured stops: page conditions where the browser stops and returns a
//! reason instead of acting. The agent relays the reason to the user through
//! its harness and retries with the user's answer.
//!
//! The classifiers live in `page-scripts/` as plain page JavaScript so every
//! engine backend (this daemon and `platform/macos`) runs the same code.

use serde_json::{json, Value};

/// Page-side classifier for irreversible controls. Evaluated with `el` bound
/// to the click target. Returns `null` or `{label, kind}`.
///
/// Matches the visible label, `value`, `aria-label`, `title`, and the
/// enclosing form's `action`. Kinds: `payment`, `send`, `delete`.
pub const IRREVERSIBLE_JS: &str = include_str!("../../../page-scripts/irreversible.js");

/// The structured stop for an unconfirmed click on an irreversible control.
pub fn irreversible_action(hit: &Value) -> (String, Value) {
    let label = hit.get("label").and_then(Value::as_str).unwrap_or("");
    let kind = hit
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("irreversible");
    (
        format!(
            "confirmation required: {kind} control {label:?}; ask the user, then retry with confirm"
        ),
        json!({
            "reason": "irreversible_action",
            "kind": kind,
            "label": label,
            "retry": { "confirm": true },
        }),
    )
}

/// Page-side classifier for a login wall. Returns `null` or
/// `{password_fields, otp_fields, signals}`.
pub const LOGIN_WALL_JS: &str = include_str!("../../../page-scripts/login-wall.js");

/// The structured stop for a detected login wall.
pub fn login_required(hit: &Value, url: &str) -> Value {
    let needs = if hit.get("otp_fields").and_then(Value::as_u64).unwrap_or(0) > 0 {
        "one_time_code"
    } else {
        "credentials"
    };
    json!({
        "reason": "login_required",
        "needs": needs,
        "url": url,
        "evidence": hit,
        "retry": { "command": "fill-login", "after": "the user provides the value" },
    })
}

/// The structured stop for a challenge the agent could not clear.
pub fn challenge_blocked(challenge: &Value) -> Value {
    json!({
        "reason": "challenge",
        "challenge_type": challenge.get("challenge_type").cloned().unwrap_or(Value::Null),
        "url": challenge.get("url").cloned().unwrap_or(Value::Null),
        "retry": { "command": "show", "after": "the user clears the challenge" },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn irreversible_stop_carries_reason_kind_and_retry() {
        let (msg, stop) =
            irreversible_action(&json!({ "label": "Place Order", "kind": "payment" }));
        assert!(msg.contains("confirmation required"));
        assert_eq!(stop["reason"], "irreversible_action");
        assert_eq!(stop["kind"], "payment");
        assert_eq!(stop["retry"]["confirm"], true);
    }

    #[test]
    fn login_stop_names_what_is_needed() {
        let stop = login_required(
            &json!({ "password_fields": 1, "otp_fields": 0 }),
            "https://x",
        );
        assert_eq!(stop["reason"], "login_required");
        assert_eq!(stop["needs"], "credentials");
        let stop = login_required(
            &json!({ "password_fields": 0, "otp_fields": 1 }),
            "https://x",
        );
        assert_eq!(stop["needs"], "one_time_code");
    }

    #[test]
    fn classifier_scripts_are_self_contained_expressions() {
        for js in [IRREVERSIBLE_JS, LOGIN_WALL_JS] {
            let t = js.trim();
            assert!(t.starts_with("(function"));
            assert!(t.ends_with(')'));
        }
    }

    /// The macOS daemon embeds generated copies of the classifiers. Run
    /// scripts/gen-page-scripts.sh when this fails.
    #[test]
    fn macos_daemon_copy_matches_shared_script() {
        let swift = include_str!("../../../platform/macos/Sources/PageScripts.swift");
        for (name, js) in [
            ("irreversibleJS", IRREVERSIBLE_JS),
            ("loginWallJS", LOGIN_WALL_JS),
            ("challengeJS", crate::automation::challenge_detector_js()),
            ("motionJS", crate::verify::MOTION_JS),
            ("targetJS", crate::automation::TARGET_JS),
            ("snapshotJS", crate::automation::SNAPSHOT_JS),
            ("contentJS", crate::coverage::CONTENT_JS),
            ("clickJS", crate::automation::CLICK_JS),
            ("typeJS", crate::automation::TYPE_JS),
            ("nativeTimeJS", crate::automation::NATIVE_TIME_JS),
            ("pressJS", crate::automation::PRESS_JS),
            ("visibilityJS", crate::automation::VISIBILITY_INSPECTOR_JS),
            ("expectJS", crate::automation::EXPECT_JS),
            ("scrollJS", crate::automation::SCROLL_JS),
            ("fillJS", crate::coverage::FILL_JS),
            ("authContextJS", crate::coverage::AUTH_CONTEXT_JS),
        ] {
            let open = format!("let {name} = #\"\"\"\n");
            let body = swift
                .split_once(open.as_str())
                .and_then(|(_, rest)| rest.split_once("\"\"\"#"))
                .map(|(body, _)| body)
                .expect("PageScripts.swift has the generated shape");
            assert_eq!(
                body, js,
                "{name} in platform/macos/Sources/PageScripts.swift is stale; run scripts/gen-page-scripts.sh"
            );
        }
    }
}
