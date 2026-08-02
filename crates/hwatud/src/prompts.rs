// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! Interactive prompts: permission requests and TLS failures, answered
//! with a y/n keypress on the bar. One pending prompt at a time per
//! window; later requests queue behind it.
//!
//! Decisions are remembered per (host, kind) for the daemon's lifetime
//! so a page asking repeatedly does not nag. Nothing is persisted:
//! restart the daemon to reset grants, which is also the safe default.

use gtk::gio;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use webkit6::prelude::*;

/// A question waiting for a y/n keypress.
pub enum Prompt {
    Permission {
        host: String,
        kind: &'static str,
        request: webkit6::PermissionRequest,
    },
    Tls {
        host: String,
        failing_uri: String,
        certificate: gio::TlsCertificate,
        reason: String,
    },
    /// The page's anti-bot provider rejected WebKitGTK.  A different engine
    /// is the only honest recovery: do not spoof hardware fingerprints or
    /// silently claim that the challenge passed.
    ExternalBrowser { uri: String },
}

impl Prompt {
    pub fn question(&self) -> String {
        match self {
            Prompt::Permission { host, kind, .. } => format!("{host} wants {kind}"),
            Prompt::Tls { host, reason, .. } => {
                format!("TLS error for {host} ({reason}), proceed?")
            }
            Prompt::ExternalBrowser { uri } => format!(
                "Cloudflare rejected WebKitGTK; open {} in another browser?",
                host_of(uri)
            ),
        }
    }

    fn memory_key(&self) -> Option<String> {
        match self {
            Prompt::Permission { host, kind, .. } => Some(format!("perm:{host}:{kind}")),
            // TLS exceptions are handled by the network session itself.
            Prompt::Tls { .. } => None,
            // Launching another application is always an explicit action.
            Prompt::ExternalBrowser { .. } => None,
        }
    }
}

/// Daemon-lifetime decision memory, shared by all windows.
pub type Memory = std::rc::Rc<RefCell<HashMap<String, bool>>>;

/// Per-window prompt queue; decision memory is daemon-wide so a grant
/// in one window covers the site in every window.
#[derive(Default)]
pub struct Prompts {
    queue: RefCell<VecDeque<Prompt>>,
    remembered: Memory,
}

impl Prompts {
    pub fn new(remembered: Memory) -> Self {
        Prompts {
            queue: RefCell::new(VecDeque::new()),
            remembered,
        }
    }

    /// Queue a prompt. Returns the question to show if the bar should
    /// open now (i.e. this prompt is at the front), or None if it was
    /// auto-answered from memory or is waiting behind another.
    pub fn push(&self, prompt: Prompt) -> Option<String> {
        if let Some(key) = prompt.memory_key() {
            if let Some(&allow) = self.remembered.borrow().get(&key) {
                answer(&prompt, allow, None);
                return None;
            }
        }
        let mut queue = self.queue.borrow_mut();
        queue.push_back(prompt);
        if queue.len() == 1 {
            Some(queue.front().unwrap().question())
        } else {
            None
        }
    }

    /// Resolve the front prompt. Returns the next question to show, if
    /// any is queued.
    pub fn answer_front(&self, allow: bool, webview: Option<&webkit6::WebView>) -> Option<String> {
        let front = self.queue.borrow_mut().pop_front()?;
        if let Some(key) = front.memory_key() {
            self.remembered.borrow_mut().insert(key, allow);
        }
        answer(&front, allow, webview);
        self.queue.borrow().front().map(|p| p.question())
    }

    pub fn has_pending(&self) -> bool {
        !self.queue.borrow().is_empty()
    }
}

fn answer(prompt: &Prompt, allow: bool, webview: Option<&webkit6::WebView>) {
    match prompt {
        Prompt::Permission { request, .. } => {
            if allow {
                request.allow();
            } else {
                request.deny();
            }
        }
        Prompt::Tls {
            host,
            failing_uri,
            certificate,
            ..
        } => {
            if !allow {
                return;
            }
            let Some(webview) = webview else { return };
            if let Some(session) = webview.network_session() {
                println!("hwatud: TLS exception for {host} (this session)");
                session.allow_tls_certificate_for_host(certificate, host);
                webview.load_uri(failing_uri);
            }
        }
        Prompt::ExternalBrowser { uri } => {
            if !allow {
                return;
            }
            match crate::external::open(uri) {
                Ok(browser) => println!("hwatud: opened {uri} in {browser}"),
                Err(error) => eprintln!("hwatud: cannot open {uri} externally: {error}"),
            }
        }
    }
}

/// Human name for a WebKit permission request subclass.
pub fn permission_kind(request: &webkit6::PermissionRequest) -> &'static str {
    use webkit6 as wk;
    if let Some(media) = request.dynamic_cast_ref::<wk::UserMediaPermissionRequest>() {
        let audio = media.is_for_audio_device();
        let video = media.is_for_video_device();
        return match (audio, video) {
            (true, true) => "camera + microphone",
            (true, false) => "microphone",
            (false, true) => "camera",
            _ => "media access",
        };
    }
    if request
        .dynamic_cast_ref::<wk::GeolocationPermissionRequest>()
        .is_some()
    {
        return "your location";
    }
    if request
        .dynamic_cast_ref::<wk::NotificationPermissionRequest>()
        .is_some()
    {
        return "notifications";
    }
    if request
        .dynamic_cast_ref::<wk::ClipboardPermissionRequest>()
        .is_some()
    {
        return "clipboard access";
    }
    if request
        .dynamic_cast_ref::<wk::PointerLockPermissionRequest>()
        .is_some()
    {
        return "pointer lock";
    }
    if request
        .dynamic_cast_ref::<wk::DeviceInfoPermissionRequest>()
        .is_some()
    {
        return "device info";
    }
    if request
        .dynamic_cast_ref::<wk::MediaKeySystemPermissionRequest>()
        .is_some()
    {
        return "DRM media keys";
    }
    if request
        .dynamic_cast_ref::<wk::WebsiteDataAccessPermissionRequest>()
        .is_some()
    {
        return "cross-site cookies";
    }
    "a permission"
}

/// `https://sub.example.com/x` -> `sub.example.com`.
pub fn host_of(uri: &str) -> String {
    glib::Uri::parse(uri, glib::UriFlags::NONE)
        .ok()
        .and_then(|u| u.host().map(|h| h.to_string()))
        .unwrap_or_else(|| uri.to_string())
}

/// Short human reason for a TLS failure.
pub fn tls_reason(flags: gio::TlsCertificateFlags) -> &'static str {
    if flags.contains(gio::TlsCertificateFlags::EXPIRED) {
        "expired certificate"
    } else if flags.contains(gio::TlsCertificateFlags::UNKNOWN_CA) {
        "unknown issuer"
    } else if flags.contains(gio::TlsCertificateFlags::BAD_IDENTITY) {
        "hostname mismatch"
    } else if flags.contains(gio::TlsCertificateFlags::NOT_ACTIVATED) {
        "certificate not yet valid"
    } else if flags.contains(gio::TlsCertificateFlags::REVOKED) {
        "revoked certificate"
    } else if flags.contains(gio::TlsCertificateFlags::INSECURE) {
        "insecure certificate"
    } else {
        "invalid certificate"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_extraction() {
        assert_eq!(host_of("https://sub.example.com/x?q=1"), "sub.example.com");
        assert_eq!(host_of("http://127.0.0.1:8080/"), "127.0.0.1");
    }

    #[test]
    fn tls_reasons() {
        assert_eq!(
            tls_reason(gio::TlsCertificateFlags::EXPIRED),
            "expired certificate"
        );
        assert_eq!(
            tls_reason(gio::TlsCertificateFlags::UNKNOWN_CA | gio::TlsCertificateFlags::EXPIRED),
            "expired certificate"
        );
        assert_eq!(
            tls_reason(gio::TlsCertificateFlags::UNKNOWN_CA),
            "unknown issuer"
        );
    }
}
