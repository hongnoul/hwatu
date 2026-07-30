// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! Per-window network request log for agent verify loops.
//!
//! An agent verifying a form submit should assert "the POST to
//! /api/charge returned 200", not squint at a success toast. This
//! module records, per window, every resource load WebKit reports
//! (`WebKitWebView::resource-load-started` + the per-resource
//! `finished`/`failed` signals): method, final URL, HTTP status,
//! an inferred resource type, and timing. `hwatu net` reads the
//! buffer; `--clear` drains it so a verification loop can diff runs.
//!
//! Where [`crate::console`] only captures *failures* (HTTP >= 400 and
//! connection errors), this records every completed request, success
//! included. Entries are pushed on completion, so their order is
//! completion order; `start_ms` carries the true start order.
//!
//! WebKitGTK limitations, stated honestly: the API exposes no request
//! destination (what Playwright calls the resource type), so `type` is
//! inferred from the response MIME type plus the main-resource flag;
//! and there is no route interception, so this is observation only.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Instant;
use webkit6::prelude::*;

/// Keep the buffer bounded; agents read recent requests, not history.
/// Long-lived windows (dev servers polling, SPAs fetching) must not
/// grow the daemon without bound.
pub const CAP: usize = 500;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Entry {
    /// HTTP method (`GET`, `POST`, ...). Non-HTTP loads (`file:`)
    /// report `GET`.
    pub method: String,
    /// Final URL of the resource (after redirects).
    pub url: String,
    /// HTTP status of the response. Absent on connection-level
    /// failures (DNS, refused, TLS) and non-HTTP loads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u32>,
    /// Inferred resource type: `document` | `stylesheet` | `script` |
    /// `image` | `font` | `media` | `data` | `other`. Inferred from
    /// the response MIME type (WebKitGTK does not expose the request
    /// destination), so a JSON API answer is `data` whether it came
    /// from fetch, XHR, or a link.
    #[serde(rename = "type")]
    pub resource_type: String,
    /// Milliseconds from the window's current top-level navigation
    /// start to this request's start. Orders requests within a page
    /// load; resets when a new document starts loading.
    pub start_ms: u64,
    /// Milliseconds from request start to completion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Error text for connection-level failures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// URL of the page that issued the request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
}

/// Bounded per-window request log. Cloned into signal closures; the
/// window owns the canonical handle, so entries survive a discard
/// (the page's state dies, the requests it made did happen).
#[derive(Clone)]
pub struct Buffer(Rc<Inner>);

struct Inner {
    queue: RefCell<VecDeque<Entry>>,
    /// Start of the current top-level navigation; `start_ms` offsets
    /// are measured from here. Reset when a main resource starts.
    epoch: Cell<Instant>,
}

impl Default for Buffer {
    fn default() -> Self {
        Buffer(Rc::new(Inner {
            queue: RefCell::new(VecDeque::new()),
            epoch: Cell::new(Instant::now()),
        }))
    }
}

impl Buffer {
    pub fn push(&self, entry: Entry) {
        let mut q = self.0.queue.borrow_mut();
        if q.len() == CAP {
            q.pop_front();
        }
        q.push_back(entry);
    }

    /// Restart the `start_ms` timeline (a new document is loading).
    fn reset_epoch(&self) {
        self.0.epoch.set(Instant::now());
    }

    /// Milliseconds since the current navigation epoch.
    fn offset_ms(&self) -> u64 {
        self.0.epoch.get().elapsed().as_millis() as u64
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

/// Infer a coarse resource type from the response MIME type. The
/// closest WebKitGTK lets us get to Playwright's resource types: the
/// request destination is not exposed on this API surface.
fn classify(mime: &str, is_main: bool) -> &'static str {
    if is_main {
        return "document";
    }
    let mime = mime.to_ascii_lowercase();
    if mime.contains("javascript") || mime.contains("ecmascript") {
        "script"
    } else if mime.starts_with("text/css") {
        "stylesheet"
    } else if mime.starts_with("image/") {
        "image"
    } else if mime.starts_with("font/") || mime.contains("font") {
        "font"
    } else if mime.starts_with("audio/") || mime.starts_with("video/") {
        "media"
    } else if mime.contains("json") || mime.contains("xml") {
        "data"
    } else if mime.starts_with("text/html") {
        // Subframe documents (iframes) answer text/html.
        "document"
    } else {
        "other"
    }
}

/// Connect a window's request log to a (freshly attached) WebView.
/// Entries are recorded when a resource finishes or fails; loads
/// cancelled by navigating away are noise, not requests worth logging.
pub fn attach(buffer: &Buffer, view: &webkit6::WebView) {
    let buffer = buffer.clone();
    view.connect_resource_load_started(move |view, resource, request| {
        let is_main = view.main_resource().as_ref() == Some(resource);
        if is_main {
            // New document: restart the start_ms timeline so offsets
            // read as "ms into this page load".
            buffer.reset_epoch();
        }
        let started = Instant::now();
        let start_ms = buffer.offset_ms();
        let method = request
            .http_method()
            .map(|m| m.to_string())
            .unwrap_or_else(|| "GET".into());
        let page = view.uri().map(|u| u.to_string());
        // `finished` also fires after `failed`; first reporter wins.
        let reported = Rc::new(Cell::new(false));
        {
            let buffer = buffer.clone();
            let reported = reported.clone();
            let method = method.clone();
            let page = page.clone();
            resource.connect_failed(move |resource, error| {
                if reported.replace(true) {
                    return;
                }
                if error.matches(gtk::gio::IOErrorEnum::Cancelled)
                    || error.to_string().to_lowercase().contains("cancelled")
                {
                    return; // navigated away mid-load: noise
                }
                buffer.push(Entry {
                    method: method.clone(),
                    url: resource.uri().map(|u| u.to_string()).unwrap_or_default(),
                    status: None,
                    resource_type: "other".into(),
                    start_ms,
                    duration_ms: Some(started.elapsed().as_millis() as u64),
                    error: Some(error.to_string()),
                    page: page.clone(),
                });
            });
        }
        {
            let buffer = buffer.clone();
            resource.connect_finished(move |resource| {
                if reported.replace(true) {
                    return;
                }
                let response = resource.response();
                let Some(response) = response else {
                    // No response and no `failed`: a cancelled load
                    // WebKit finished silently. Not a request.
                    return;
                };
                let mime = response.mime_type().unwrap_or_default();
                buffer.push(Entry {
                    method: method.clone(),
                    url: resource.uri().map(|u| u.to_string()).unwrap_or_default(),
                    status: Some(response.status_code()).filter(|&s| s != 0),
                    resource_type: classify(&mime, is_main).into(),
                    start_ms,
                    duration_ms: Some(started.elapsed().as_millis() as u64),
                    error: None,
                    page: page.clone(),
                });
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{classify, Buffer, Entry, CAP};

    fn entry(url: &str) -> Entry {
        Entry {
            method: "GET".into(),
            url: url.into(),
            status: Some(200),
            resource_type: "other".into(),
            start_ms: 0,
            duration_ms: Some(1),
            error: None,
            page: None,
        }
    }

    #[test]
    fn buffer_caps_and_limits() {
        let b = Buffer::default();
        for i in 0..(CAP + 10) {
            b.push(entry(&format!("u{i}")));
        }
        let all = b.read(false, None);
        assert_eq!(all.len(), CAP);
        assert_eq!(all.first().unwrap().url, "u10"); // oldest dropped
        let last2 = b.read(false, Some(2));
        assert_eq!(last2.len(), 2);
        assert_eq!(last2[1].url, format!("u{}", CAP + 9));
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
    fn classify_covers_common_mimes() {
        assert_eq!(classify("text/html", true), "document");
        assert_eq!(classify("text/html", false), "document"); // iframe
        assert_eq!(classify("text/css", false), "stylesheet");
        assert_eq!(classify("application/javascript", false), "script");
        assert_eq!(classify("text/javascript; charset=utf-8", false), "script");
        assert_eq!(classify("image/png", false), "image");
        assert_eq!(classify("font/woff2", false), "font");
        assert_eq!(classify("application/font-woff", false), "font");
        assert_eq!(classify("video/mp4", false), "media");
        assert_eq!(classify("application/json", false), "data");
        assert_eq!(classify("application/octet-stream", false), "other");
    }

    #[test]
    fn entry_serializes_type_and_omits_absent_fields() {
        let wire = serde_json::to_string(&entry("u")).unwrap();
        assert!(wire.contains("\"type\":\"other\""));
        assert!(!wire.contains("error"));
        assert!(!wire.contains("page"));
    }
}
