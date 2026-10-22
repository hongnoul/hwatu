// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Justin Hong
//! Structured stops: page conditions where the browser stops and returns a
//! reason instead of acting. The agent relays the reason to the user through
//! its harness and retries with the user's answer.
//!
//! The classifiers are plain page JavaScript so every engine backend runs
//! the same code.

use serde_json::{json, Value};

/// Page-side classifier for irreversible controls. Evaluated with `el` bound
/// to the click target. Returns `null` or `{label, kind}`.
///
/// Matches the visible label, `value`, `aria-label`, `title`, and the
/// enclosing form's `action`. Kinds: `payment`, `send`, `delete`.
pub const IRREVERSIBLE_JS: &str = r#"
(function (el) {
  const control = el.closest('button, a, input, [role="button"], [role="menuitem"], [role="link"]') || el;
  const parts = [
    control.innerText || control.textContent || '',
    control.value || '',
    control.getAttribute('aria-label') || '',
    control.getAttribute('title') || '',
  ];
  const label = parts.join(' ').replace(/\s+/g, ' ').trim().slice(0, 160);
  const text = label.toLowerCase();
  const form = control.form || control.closest('form');
  const action = ((form && form.getAttribute('action')) || '').toLowerCase();
  const rules = [
    ['payment', /\b(place (your )?order|pay( now)?|complete (purchase|order|payment)|confirm (purchase|order|payment)|buy now|submit order|checkout and pay|purchase|subscribe now|start (my )?subscription|donate)\b/],
    ['send', /\b(send( now| message| email| money| payment)?|post( now)?|publish|submit application|transfer)\b/],
    ['delete', /\b(delete|remove (account|permanently)|permanently delete|erase|close account|cancel (my )?(account|subscription))\b/],
  ];
  for (const [kind, re] of rules) {
    if (re.test(text)) return { label, kind };
  }
  if (/\/(checkout|payment|purchase|order)\b.*\b(submit|place|confirm|complete)/.test(action)) {
    return { label, kind: 'payment' };
  }
  return null;
})(el)
"#;

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
pub const LOGIN_WALL_JS: &str = r#"
(function () {
  const visible = e => {
    const r = e.getBoundingClientRect();
    const s = getComputedStyle(e);
    return r.width > 0 && r.height > 0 && s.visibility !== 'hidden' && s.display !== 'none';
  };
  const pw = [...document.querySelectorAll('input[type="password"]')].filter(visible).length;
  const otp = [...document.querySelectorAll('input[autocomplete="one-time-code"], input[name*="otp" i], input[name*="code" i][inputmode="numeric"]')].filter(visible).length;
  const text = (document.title + ' ' + (document.body ? document.body.innerText.slice(0, 4000) : '')).toLowerCase();
  const signals = [];
  if (/\b(sign in|log in|login)\b/.test(text)) signals.push('sign_in_text');
  if (/\b(verification code|one-time code|enter the code)\b/.test(text)) signals.push('code_text');
  if (/\/(login|signin|sign-in|auth|sso)\b/i.test(location.pathname)) signals.push('login_path');
  if (pw === 0 && otp === 0 && !signals.includes('login_path')) return null;
  return { password_fields: pw, otp_fields: otp, signals };
})()
"#;

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
}
