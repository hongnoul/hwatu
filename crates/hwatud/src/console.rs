// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! Per-window console / error / network capture for agent verify loops.
//!
//! "Why is this page broken" is unanswerable from a screenshot. This
//! module buffers, per window: `console.*` calls, uncaught exceptions,
//! unhandled promise rejections (captured by a user script injected at
//! document start), plus failed resource loads and HTTP >= 400
//! responses (captured on the Rust side from WebKit's resource
//! signals). `hwatu console` reads the buffer; `--clear` drains it so
//! a verification loop can diff runs.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use webkit6::prelude::*;

/// Script message handler name the user script posts to.
const HANDLER: &str = "hwatu_console";

/// Keep the buffer bounded; agents read recent entries, not history.
const CAP: usize = 500;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Entry {
    /// Milliseconds since the daemon captured the entry's epoch
    /// (UNIX time). Lets an agent order entries across reads.
    /// Stamped daemon-side; the capture script does not send it.
    #[serde(default)]
    pub ts_ms: u64,
    /// `console` | `exception` | `network`.
    pub kind: String,
    /// console level (`log`/`info`/`warn`/`error`/`debug`), or
    /// `error` for exceptions and network failures.
    pub level: String,
    pub text: String,
    /// Source URL: the script file for exceptions, the resource for
    /// network entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u64>,
    /// HTTP status for network entries (absent on connection failures).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u32>,
    /// URL of the page that produced the entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
}

/// True when Cloudflare says its challenge cannot run in this browser.
///
/// 110500 is the documented unsupported-browser error.  The 600 family is a
/// generic execution rejection, and is what Turnstile currently emits after
/// detecting WebKitGTK's deliberately generic WebGL fingerprint.
pub fn is_turnstile_compat_error(entry: &Entry) -> bool {
    entry.kind == "console"
        && entry.text.contains("[Cloudflare Turnstile] Error:")
        && (entry.text.contains("Error: 110500") || entry.text.contains("Error: 600"))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Bounded per-window entry queue. Cloned into signal closures; the
/// window owns the canonical handle, so entries survive a discard
/// (the page's own state doesn't, but what it logged did happen).
/// An optional hook observes every push (push-IPC event fan-out).
#[derive(Clone, Default)]
pub struct Buffer(Rc<Inner>);

/// Observer invoked on every buffered entry (push-IPC fan-out).
type Hook = Rc<dyn Fn(&Entry)>;

#[derive(Default)]
struct Inner {
    queue: RefCell<VecDeque<Entry>>,
    hook: RefCell<Option<Hook>>,
}

impl Buffer {
    pub fn push(&self, entry: Entry) {
        {
            let mut q = self.0.queue.borrow_mut();
            if q.len() == CAP {
                q.pop_front();
            }
            q.push_back(entry.clone());
        }
        // Hook after the buffer write, outside the borrow: the hook
        // fans out to IPC subscribers and must not re-enter a held
        // borrow.
        let hook = self.0.hook.borrow().clone();
        if let Some(hook) = hook {
            hook(&entry);
        }
    }

    /// Observe every future push (one hook per buffer; the window
    /// installs it once at build time).
    pub fn set_hook(&self, hook: impl Fn(&Entry) + 'static) {
        self.0.hook.replace(Some(Rc::new(hook)));
    }

    /// Read the last `limit` entries (all when `None`); `clear` drains
    /// the whole buffer after reading.
    pub fn read(&self, clear: bool, limit: Option<usize>) -> Vec<Entry> {
        let mut q = self.0.queue.borrow_mut();
        let skip = limit.map_or(0, |n| q.len().saturating_sub(n));
        let out = q.iter().skip(skip).cloned().collect();
        if clear {
            q.clear();
        }
        out
    }
}

/// User script injected at document start in every frame: wraps
/// `console.*`, hooks `error` and `unhandledrejection`, and posts
/// entries to the daemon. Idempotent per realm.
const CAPTURE_JS: &str = r#"(() => {
  if (window.__hwatu_console_wired) return;
  window.__hwatu_console_wired = true;
  const send = (entry) => {
    try { window.webkit.messageHandlers.hwatu_console.postMessage(JSON.stringify(entry)); }
    catch (e) {}
  };
  const fmt = (args) => args.map((a) => {
    if (typeof a === 'string') return a;
    if (a instanceof Error) return a.message + (a.stack ? '\n' + a.stack : '');
    try { return JSON.stringify(a); } catch (e) { return String(a); }
  }).join(' ').slice(0, 2000);
  for (const level of ['log', 'info', 'warn', 'error', 'debug']) {
    const orig = console[level];
    console[level] = function (...args) {
      send({ kind: 'console', level, text: fmt(args) });
      return orig.apply(this, args);
    };
  }
  // capture=true would also see resource errors (dead <img> etc.),
  // which have no message; those are captured natively instead.
  window.addEventListener('error', (e) => {
    if (!e.message) return;
    send({ kind: 'exception', level: 'error', text: String(e.message).slice(0, 2000),
           url: e.filename || undefined, line: e.lineno || undefined });
  });
  window.addEventListener('unhandledrejection', (e) => {
    let reason;
    if (e.reason instanceof Error) reason = e.reason.message + (e.reason.stack ? '\n' + e.reason.stack : '');
    else { try { reason = JSON.stringify(e.reason); } catch (err) { reason = String(e.reason); } }
    send({ kind: 'exception', level: 'error',
           text: ('unhandled rejection: ' + reason).slice(0, 2000) });
  });
})();"#;

/// Register the capture user script + message handler on a WebView's
/// content manager. Must run on every view (prewarmed pool, popups)
/// before it loads page content.
pub fn wire_view(view: &webkit6::WebView) {
    let Some(ucm) = view.user_content_manager() else {
        return;
    };
    ucm.register_script_message_handler(HANDLER, None);
    let script = webkit6::UserScript::new(
        CAPTURE_JS,
        webkit6::UserContentInjectedFrames::AllFrames,
        webkit6::UserScriptInjectionTime::Start,
        &[],
        &[],
    );
    ucm.add_script(&script);
}

/// Connect a window's buffer to a (freshly attached) WebView: script
/// messages from the capture script, plus native resource-load
/// failures and HTTP >= 400 responses.
pub fn attach(buffer: &Buffer, view: &webkit6::WebView) {
    if let Some(ucm) = view.user_content_manager() {
        let buffer = buffer.clone();
        let view = view.clone();
        ucm.connect_script_message_received(Some(HANDLER), move |_, value| {
            let raw = value.to_str();
            let Ok(mut entry) = serde_json::from_str::<Entry>(&raw) else {
                return;
            };
            entry.ts_ms = now_ms();
            entry.page = view.uri().map(|u| u.to_string());
            buffer.push(entry);
        });
    }

    let buffer = buffer.clone();
    view.connect_resource_load_started(move |view, resource, _request| {
        let page = view.uri().map(|u| u.to_string());
        // Connection-level failures (DNS, refused, TLS). Loads
        // cancelled by navigating away are noise, not errors.
        {
            let buffer = buffer.clone();
            let page = page.clone();
            resource.connect_failed(move |resource, error| {
                if error.matches(gtk::gio::IOErrorEnum::Cancelled)
                    || error.to_string().to_lowercase().contains("cancelled")
                {
                    return;
                }
                buffer.push(Entry {
                    ts_ms: now_ms(),
                    kind: "network".into(),
                    level: "error".into(),
                    text: error.to_string(),
                    url: resource.uri().map(|u| u.to_string()),
                    line: None,
                    status: None,
                    page: page.clone(),
                });
            });
        }
        // HTTP errors: the load "succeeded" at the transport level but
        // the server said no. 404 assets and 500 API answers live here.
        {
            let buffer = buffer.clone();
            resource.connect_finished(move |resource| {
                let Some(response) = resource.response() else {
                    return;
                };
                let status = response.status_code();
                if status < 400 {
                    return;
                }
                buffer.push(Entry {
                    ts_ms: now_ms(),
                    kind: "network".into(),
                    level: "error".into(),
                    text: format!("HTTP {status}"),
                    url: resource.uri().map(|u| u.to_string()),
                    line: None,
                    status: Some(status),
                    page: page.clone(),
                });
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{is_turnstile_compat_error, Buffer, Entry, CAP};

    fn entry(text: &str) -> Entry {
        Entry {
            ts_ms: 0,
            kind: "console".into(),
            level: "log".into(),
            text: text.into(),
            url: None,
            line: None,
            status: None,
            page: None,
        }
    }

    #[test]
    fn buffer_caps_and_limits() {
        let b = Buffer::default();
        for i in 0..(CAP + 10) {
            b.push(entry(&format!("m{i}")));
        }
        let all = b.read(false, None);
        assert_eq!(all.len(), CAP);
        assert_eq!(all.first().unwrap().text, "m10"); // oldest dropped
        let last2 = b.read(false, Some(2));
        assert_eq!(last2.len(), 2);
        assert_eq!(last2[1].text, format!("m{}", CAP + 9));
    }

    #[test]
    fn clear_drains_after_read() {
        let b = Buffer::default();
        b.push(entry("a"));
        b.push(entry("b"));
        assert_eq!(b.read(true, Some(1)).len(), 1);
        assert!(b.read(false, None).is_empty());
    }

    #[test]
    fn detects_turnstile_browser_rejections_only() {
        let mut e = entry("[Cloudflare Turnstile] Error: 600010.");
        assert!(is_turnstile_compat_error(&e));
        e.text = "[Cloudflare Turnstile] Error: 110500.".into();
        assert!(is_turnstile_compat_error(&e));
        e.text = "unrelated API Error: 600010".into();
        assert!(!is_turnstile_compat_error(&e));
        e.text = "[Cloudflare Turnstile] Error: 200500.".into();
        assert!(!is_turnstile_compat_error(&e));
    }
}
