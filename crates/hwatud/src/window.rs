//! Browser window: one WebView per toplevel, zero chrome.
//! The tiling WM is the tab bar.
//!
//! RAM strategy: when a window stays unfocused past a timeout, its
//! WebView is *discarded*: navigation history is serialized into a
//! `WebViewSessionState` blob, the WebView (and with it the web
//! process) is destroyed, and a placeholder widget takes its place.
//! On focus, a prewarmed WebView is adopted and the state restored, so
//! resume feels instant.

use crate::Daemon;
use gtk::prelude::*;
use hwatu_ipc::WindowInfo;
use std::cell::RefCell;
use std::rc::Rc;
use webkit6::prelude::*;

/// Seconds an unfocused window keeps its live WebView. Override with
/// HWATU_DISCARD_SECS; 0 disables discarding.
fn discard_timeout_secs() -> u64 {
    std::env::var("HWATU_DISCARD_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(120)
}

/// Page a bare `hwatu` opens. Override with HWATU_HOME (any URL, or
/// `about:blank`); defaults to the hwatu site.
fn home_page() -> String {
    std::env::var("HWATU_HOME")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "https://hongnoul.github.io/hwatu/".into())
}

/// State saved across a discard.
struct SavedState {
    session: Option<webkit6::WebViewSessionState>,
    url: String,
    title: String,
}

pub struct BrowserWindow {
    pub id: u64,
    pub window: gtk::Window,
    daemon: Rc<Daemon>,
    webview: RefCell<Option<webkit6::WebView>>,
    saved: RefCell<Option<SavedState>>,
    discard_timer: RefCell<Option<glib::SourceId>>,
}

/// Scroll-critical engine features, enabled if this WebKit build has
/// them. All of these default on in current WebKit but are Embedder/
/// Internal status, so distros can and do flip them; pinning them here
/// makes smoothness a property of hwatu rather than of the distro.
/// Unknown identifiers are skipped, so this degrades gracefully across
/// WebKitGTK versions (enable-what-exists, never version-match).
const SCROLL_FEATURES: &[&str] = &[
    "AsyncFrameScrolling",    // main-frame scrolling off the main thread
    "AsyncOverflowScrolling", // same for overflow: scroll subtrees
    "ThreadedScrolling",      // compositor-thread scroll updates
];

/// `HWATU_WEBKIT_FEATURES=Ident:on,Other:off` — escape hatch for odd
/// hardware. Applied last, so it can override anything above.
fn feature_overrides() -> Vec<(String, bool)> {
    std::env::var("HWATU_WEBKIT_FEATURES")
        .map(|raw| parse_feature_overrides(&raw))
        .unwrap_or_default()
}

fn parse_feature_overrides(raw: &str) -> Vec<(String, bool)> {
    raw.split(',')
        .filter_map(|entry| {
            let (ident, val) = entry.split_once(':')?;
            let on = match val.trim() {
                "on" | "true" | "1" => true,
                "off" | "false" | "0" => false,
                _ => return None,
            };
            Some((ident.trim().to_string(), on))
        })
        .collect()
}

/// Build a fully configured WebView. Called for the prewarm pool and as
/// a fallback; all engine knobs live here — never on the spawn path.
pub fn build_webview() -> webkit6::WebView {
    let view = webkit6::WebView::new();
    if let Some(settings) = webkit6::prelude::WebViewExt::settings(&view) {
        settings.set_enable_developer_extras(true);
        // Render as intended: leave JS, media, canvas, webgl at defaults.
        settings.set_enable_page_cache(true); // bfcache

        // Scrolling must hit the GPU compositor path. The default
        // ("on demand") drops simple pages to CPU raster, and CPU
        // raster is where scroll jank lives.
        settings.set_hardware_acceleration_policy(webkit6::HardwareAccelerationPolicy::Always);
        // Animate discrete wheel ticks; precise touchpad deltas are
        // unaffected.
        settings.set_enable_smooth_scrolling(true);

        let overrides = feature_overrides();
        if let Some(features) = webkit6::Settings::all_features() {
            for i in 0..features.length() {
                let Some(feature) = features.get(i) else {
                    continue;
                };
                let ident = feature.identifier().unwrap_or_default();
                if SCROLL_FEATURES.contains(&ident.as_str()) {
                    settings.set_feature_enabled(&feature, true);
                }
                if let Some((_, on)) = overrides.iter().find(|(name, _)| *name == ident) {
                    settings.set_feature_enabled(&feature, *on);
                }
            }
        }
    }
    view
}

impl BrowserWindow {
    pub fn open(daemon: &Rc<Daemon>, url: Option<String>, app_id: Option<String>) -> WindowInfo {
        let id = daemon.alloc_id();
        let webview = daemon.take_webview();

        let window = gtk::Window::builder()
            .application(&daemon.app)
            .default_width(1024)
            .default_height(768)
            .title("hwatu")
            .build();

        // Tiling WMs key window rules off app_id / WM_CLASS. GTK derives
        // it from the application id; per-window app_id overrides are
        // post-MVP. Recorded for `hwatu list` semantics later.
        let _ = &app_id;

        let target = url.unwrap_or_else(home_page);

        let this = Rc::new(BrowserWindow {
            id,
            window: window.clone(),
            daemon: daemon.clone(),
            webview: RefCell::new(None),
            saved: RefCell::new(None),
            discard_timer: RefCell::new(None),
        });

        this.attach_webview(webview.clone());
        webview.load_uri(&target);

        // Ctrl+q closes the window; the daemon (and engine) stay warm.
        {
            let ctrl = gtk::EventControllerKey::new();
            let win = window.clone();
            ctrl.connect_key_pressed(move |_, key, _, state| {
                if state.contains(gtk::gdk::ModifierType::CONTROL_MASK) && key == gtk::gdk::Key::q {
                    win.close();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            window.add_controller(ctrl);
        }

        // Focus-driven lifecycle: unfocused windows are scheduled for
        // discard, focused ones are restored immediately.
        {
            let this = this.clone();
            window.connect_is_active_notify(move |win| {
                if win.is_active() {
                    this.cancel_discard_timer();
                    this.restore();
                } else {
                    this.schedule_discard();
                }
            });
        }

        // Drop from the registry when the WM closes us.
        {
            let daemon = daemon.clone();
            let this = this.clone();
            window.connect_close_request(move |_| {
                this.cancel_discard_timer();
                daemon.windows.borrow_mut().remove(&id);
                glib::Propagation::Proceed
            });
        }

        window.present();

        let info = WindowInfo {
            id,
            url: target.clone(),
            title: String::new(),
            suspended: false,
        };
        daemon.windows.borrow_mut().insert(id, this);
        info
    }

    /// Put a WebView into the window and wire its signals.
    fn attach_webview(self: &Rc<Self>, webview: webkit6::WebView) {
        let win = self.window.clone();
        webview.connect_title_notify(move |wv| {
            let title = wv.title().unwrap_or_default();
            win.set_title(Some(if title.is_empty() {
                "hwatu"
            } else {
                title.as_str()
            }));
        });
        self.window.set_child(Some(&webview));
        self.webview.replace(Some(webview));
    }

    fn schedule_discard(self: &Rc<Self>) {
        let secs = discard_timeout_secs();
        if secs == 0 || self.webview.borrow().is_none() {
            return;
        }
        self.cancel_discard_timer();
        let this = self.clone();
        let source =
            glib::timeout_add_local_once(std::time::Duration::from_secs(secs), move || {
                this.discard_timer.replace(None);
                this.discard();
            });
        self.discard_timer.replace(Some(source));
    }

    fn cancel_discard_timer(&self) {
        if let Some(source) = self.discard_timer.borrow_mut().take() {
            source.remove();
        }
    }

    /// Serialize state, destroy the WebView, show a placeholder.
    fn discard(self: &Rc<Self>) {
        // Never discard a focused or loading window.
        if self.window.is_active() {
            return;
        }
        let Some(webview) = self.webview.borrow_mut().take() else {
            return;
        };
        if webview.is_loading() {
            // Try again later rather than losing an in-flight load.
            self.webview.replace(Some(webview));
            self.schedule_discard();
            return;
        }

        let url = webview.uri().map(|u| u.to_string()).unwrap_or_default();
        let title = webview.title().map(|t| t.to_string()).unwrap_or_default();
        self.saved.replace(Some(SavedState {
            session: webview.session_state(),
            url,
            title: title.clone(),
        }));

        let placeholder = gtk::Label::builder()
            .label(if title.is_empty() {
                "(suspended)"
            } else {
                &title
            })
            .build();
        self.window.set_child(Some(&placeholder));
        // Dropping the WebView alone is not enough: WebKit keeps the web
        // process cached for reuse. State is already serialized, so kill
        // the process outright; that is where the RAM comes back.
        webview.terminate_web_process();
        drop(webview);
    }

    /// Bring a discarded window back to life from the prewarm pool.
    fn restore(self: &Rc<Self>) {
        if self.webview.borrow().is_some() {
            return;
        }
        let Some(saved) = self.saved.borrow_mut().take() else {
            return;
        };
        let webview = self.daemon.take_webview();
        if let Some(state) = &saved.session {
            webview.restore_session_state(state);
        }
        // restore_session_state rebuilds history but does not navigate;
        // drive it to the current item (or fall back to the raw URL).
        let current = webview.back_forward_list().and_then(|l| l.current_item());
        match current {
            Some(item) => webview.go_to_back_forward_list_item(&item),
            None if !saved.url.is_empty() => webview.load_uri(&saved.url),
            None => {}
        }
        self.attach_webview(webview);
    }

    pub fn present(&self) {
        self.window.present();
    }

    pub fn info(&self) -> WindowInfo {
        match &*self.webview.borrow() {
            Some(wv) => WindowInfo {
                id: self.id,
                url: wv.uri().map(|u| u.to_string()).unwrap_or_default(),
                title: wv.title().map(|t| t.to_string()).unwrap_or_default(),
                suspended: false,
            },
            None => {
                let saved = self.saved.borrow();
                let (url, title) = saved
                    .as_ref()
                    .map(|s| (s.url.clone(), s.title.clone()))
                    .unwrap_or_default();
                WindowInfo {
                    id: self.id,
                    url,
                    title,
                    suspended: true,
                }
            }
        }
    }

    pub fn close(&self) {
        self.window.close();
    }
}

#[cfg(test)]
mod tests {
    use super::parse_feature_overrides;

    #[test]
    fn parses_on_off_pairs() {
        assert_eq!(
            parse_feature_overrides("ThreadedScrolling:off, AsyncFrameScrolling:on"),
            vec![
                ("ThreadedScrolling".to_string(), false),
                ("AsyncFrameScrolling".to_string(), true),
            ]
        );
    }

    #[test]
    fn skips_malformed_entries() {
        assert_eq!(
            parse_feature_overrides("NoColon,Bad:maybe,Good:1"),
            vec![("Good".to_string(), true)]
        );
    }

    #[test]
    fn empty_input_is_empty() {
        assert!(parse_feature_overrides("").is_empty());
    }
}
