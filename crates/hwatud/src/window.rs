//! Browser window: one WebView per toplevel, zero chrome.
//! The tiling WM is the tab bar.
//!
//! RAM strategy: when a window stays unfocused past a timeout, its
//! WebView is *discarded*: navigation history is serialized into a
//! `WebViewSessionState` blob, the WebView (and with it the web
//! process) is destroyed, and a placeholder widget takes its place.
//! On focus, a prewarmed WebView is adopted and the state restored, so
//! resume feels instant.

use crate::bar::{Bar, BarMode};
use crate::keys;
use crate::prompts::{self, Prompt, Prompts};
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

/// State saved across a discard. The session blob itself lives on disk
/// (see [`discard_dir`]): keeping it in RAM would leak per-window
/// memory exactly when the point of discarding is to reclaim it, and
/// heavy pages can serialize surprisingly large histories.
struct SavedState {
    /// Path of the serialized `WebViewSessionState`, if writing it out
    /// succeeded. `None` falls back to a plain URL reload on restore.
    session_file: Option<std::path::PathBuf>,
    url: String,
    title: String,
}

/// Where discarded session blobs live: `~/.cache/hwatu/discard`.
/// Deliberately NOT `$XDG_RUNTIME_DIR`, which is tmpfs (RAM-backed) on
/// every mainstream distro and would defeat the reclaim. Files are
/// removed on restore, on window close, and swept at daemon startup.
fn discard_dir() -> Option<std::path::PathBuf> {
    let base = std::env::var("XDG_CACHE_HOME")
        .ok()
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| std::path::PathBuf::from(h).join(".cache"))
        })?;
    let dir = base.join("hwatu").join("discard");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// Remove blobs orphaned by a previous daemon (crash, SIGKILL). Call
/// once at startup, before any window can discard.
pub fn sweep_discard_dir() {
    let Some(dir) = discard_dir() else { return };
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

pub struct BrowserWindow {
    pub id: u64,
    pub window: gtk::Window,
    daemon: Rc<Daemon>,
    /// The webview sits inside this overlay; the bar floats above it.
    overlay: gtk::Overlay,
    bar: Bar,
    prompts: Prompts,
    webview: RefCell<Option<webkit6::WebView>>,
    saved: RefCell<Option<SavedState>>,
    discard_timer: RefCell<Option<glib::SourceId>>,
    /// Marker shared between windows whose WebViews share one web
    /// process (popup ↔ opener, via `related_view`). `discard` must
    /// not terminate a process another live window still uses;
    /// `Rc::strong_count > 1` is that test. Cleared on discard: a
    /// restored view comes from the pool and is no longer related.
    process_group: RefCell<Option<Rc<()>>>,
    /// Wayland app_id this window was opened with, echoed by `hwatu list`.
    app_id: Option<String>,
}

/// `HWATU_WEBKIT_FEATURES=Ident:on,Other:off` — escape hatch for odd
/// hardware. hwatu used to force-enable the async/threaded scrolling
/// features here, but forcing them breaks wheel scrolling outright on
/// some driver stacks (notably NVIDIA + Wayland), so engine defaults
/// now rule and this env var is the only way to flip features.
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

/// Set the Wayland xdg_toplevel app_id for one window, overriding the
/// GTK-derived application id. No-op on X11 (WM_CLASS stays global to
/// the GTK app; per-window class on X11 is not worth the unsafe Xlib).
fn set_wayland_app_id(window: &gtk::Window, app_id: &str) {
    use gtk::prelude::*;
    let Some(surface) = window.surface() else {
        return;
    };
    if let Some(toplevel) = surface.dynamic_cast_ref::<gdk_wayland::WaylandToplevel>() {
        toplevel.set_application_id(app_id);
    }
}

/// Build a fully configured WebView. Called for the prewarm pool and as
/// a fallback; all engine knobs live here — never on the spawn path.
pub fn build_webview() -> webkit6::WebView {
    let view = webkit6::WebView::new();
    apply_view_settings(&view);
    view
}

/// Shared engine settings, applied to prewarmed views and to popup
/// views built with `related_view` (which bypass `build_webview`).
fn apply_view_settings(view: &webkit6::WebView) {
    if let Some(settings) = webkit6::prelude::WebViewExt::settings(view) {
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
        if !overrides.is_empty() {
            if let Some(features) = webkit6::Settings::all_features() {
                for i in 0..features.length() {
                    let Some(feature) = features.get(i) else {
                        continue;
                    };
                    let ident = feature.identifier().unwrap_or_default();
                    if let Some((_, on)) = overrides.iter().find(|(name, _)| *name == ident) {
                        settings.set_feature_enabled(&feature, *on);
                    }
                }
            }
        }
    }
}

impl BrowserWindow {
    pub fn open(daemon: &Rc<Daemon>, url: Option<String>, app_id: Option<String>) -> WindowInfo {
        let webview = daemon.take_webview();
        let target = url.unwrap_or_else(home_page);
        let this = Self::build(daemon, webview.clone(), app_id.clone());
        webview.load_uri(&target);
        this.window.present();
        WindowInfo {
            id: this.id,
            url: target,
            title: String::new(),
            focused: false,
            suspended: false,
            app_id,
        }
    }

    /// Open a window for a popup requested by the page (`window.open`,
    /// `target=_blank`). The new WebView must be built with
    /// `related_view` so it shares the opener's web process —
    /// `window.opener` and postMessage (OAuth flows) depend on it. The
    /// prewarmed pool can't serve this, so the view is built here.
    /// WebKit drives the navigation itself; loading anything manually
    /// would break the popup contract. The window is presented on
    /// ready-to-show, once the engine has applied window features.
    fn open_popup(self: &Rc<Self>, related: &webkit6::WebView) -> webkit6::WebView {
        let webview = webkit6::WebView::builder().related_view(related).build();
        apply_view_settings(&webview);
        self.daemon.adblock.apply_to(&webview);
        let popup = Self::build(&self.daemon, webview.clone(), self.app_id.clone());
        // Mark both windows as sharing a web process so neither
        // discard() terminates it while the other still needs it.
        let group = self
            .process_group
            .borrow_mut()
            .get_or_insert_with(|| Rc::new(()))
            .clone();
        popup.process_group.replace(Some(group));
        {
            let win = popup.window.clone();
            webview.connect_ready_to_show(move |_| win.present());
        }
        webview
    }

    /// Shared window construction: chrome, keys, lifecycle, registry.
    /// Callers decide how the WebView gets its content (load_uri for
    /// normal opens, engine-driven for popups) and when to present.
    fn build(
        daemon: &Rc<Daemon>,
        webview: webkit6::WebView,
        app_id: Option<String>,
    ) -> Rc<Self> {
        let id = daemon.alloc_id();

        let window = gtk::Window::builder()
            .application(&daemon.app)
            .default_width(1024)
            .default_height(768)
            .title("hwatu")
            .build();

        // Tiling WMs key window rules off app_id / WM_CLASS. GTK derives
        // the default from the application id; a per-window override
        // lets rules target windows individually (`hwatu --app-id mail
        // gmail.com` + `windowrule = workspace 3, class:mail`).
        // gdk_wayland_toplevel_set_application_id is a silent no-op
        // until the xdg_toplevel exists, which happens at map (inside
        // present), so set it from an idle callback queued on map.
        if let Some(app_id) = app_id.clone() {
            window.connect_map(move |win| {
                let win = win.clone();
                let app_id = app_id.clone();
                glib::idle_add_local_once(move || set_wayland_app_id(&win, &app_id));
            });
        }

        let overlay = gtk::Overlay::new();
        let bar = Bar::new();
        overlay.add_overlay(&bar.root);
        window.set_child(Some(&overlay));

        let this = Rc::new(BrowserWindow {
            id,
            window: window.clone(),
            daemon: daemon.clone(),
            overlay,
            bar,
            prompts: Prompts::new(daemon.prompt_memory.clone()),
            webview: RefCell::new(None),
            saved: RefCell::new(None),
            discard_timer: RefCell::new(None),
            process_group: RefCell::new(None),
            app_id,
        });

        this.attach_webview(webview);
        this.wire_bar();

        // Ctrl+q closes the window; the daemon (and engine) stay warm.
        {
            let ctrl = gtk::EventControllerKey::new();
            let this2 = this.clone();
            ctrl.connect_key_pressed(move |_, key, _, state| this2.on_window_key(key, state));
            window.add_controller(ctrl);
        }

        // Capture-phase keys: things that must win over the page, which
        // keeps focus and would otherwise swallow them before bubble.
        // Modified chords (ctrl/alt) dispatch here via the keymap;
        // y/n/Esc while the bar is in confirm mode are fixed keys.
        {
            let ctrl = gtk::EventControllerKey::new();
            ctrl.set_propagation_phase(gtk::PropagationPhase::Capture);
            let this2 = this.clone();
            ctrl.connect_key_pressed(move |_, key, _, state| {
                // While the bar's entry owns focus (find/URL typing),
                // global chords stay out of the way: Ctrl+o in a URL
                // prompt must not navigate history under the bar.
                let entry_open = matches!(this2.bar.mode(), BarMode::Find { .. } | BarMode::Url);
                if !entry_open {
                    if let Some(action) =
                        this2.daemon.keymap.lookup(keys::Phase::Capture, key, state)
                    {
                        return this2.run_action(action);
                    }
                }
                let BarMode::Confirm { tag } = this2.bar.mode() else {
                    return glib::Propagation::Proceed;
                };
                match key {
                    gtk::gdk::Key::y | gtk::gdk::Key::Y => this2.answer_confirm(&tag, true),
                    gtk::gdk::Key::n | gtk::gdk::Key::N | gtk::gdk::Key::Escape => {
                        this2.answer_confirm(&tag, false)
                    }
                    _ => {}
                }
                glib::Propagation::Stop
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
        // A window that opens in the background (WM rule, focus
        // elsewhere) never fires the active->inactive transition; arm
        // the timer at birth. discard() rechecks is_active, and gaining
        // focus cancels it, so a focused window is never affected.
        this.schedule_discard();

        // Drop from the registry when the WM closes us.
        {
            let daemon = daemon.clone();
            let this = this.clone();
            window.connect_close_request(move |_| {
                this.cancel_discard_timer();
                // A discarded window closed for good: its blob is dead.
                if let Some(saved) = this.saved.borrow_mut().take() {
                    if let Some(path) = saved.session_file {
                        let _ = std::fs::remove_file(path);
                    }
                }
                daemon.windows.borrow_mut().remove(&id);
                daemon.schedule_session_save();
                glib::Propagation::Proceed
            });
        }

        daemon.windows.borrow_mut().insert(id, this.clone());
        daemon.schedule_session_save();
        this
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
        // Every navigation refreshes the crash-recovery snapshot
        // (debounced in the daemon).
        {
            let daemon = self.daemon.clone();
            webview.connect_uri_notify(move |_| daemon.schedule_session_save());
        }
        crate::downloads::wire_session(&self.daemon, &webview);
        // Permission requests (mic/cam/location/...) become bar
        // prompts; remembered decisions answer silently.
        {
            let this = self.clone();
            webview.connect_permission_request(move |wv, request| {
                let host = prompts::host_of(&wv.uri().unwrap_or_default());
                let kind = prompts::permission_kind(request);
                this.push_prompt(Prompt::Permission {
                    host,
                    kind,
                    request: request.clone(),
                });
                true // handled; do not fall back to default deny-now
            });
        }
        // TLS failures: explain in the bar, y adds a per-host
        // exception for this session and reloads.
        {
            let this = self.clone();
            webview.connect_load_failed_with_tls_errors(move |_, failing_uri, cert, flags| {
                this.push_prompt(Prompt::Tls {
                    host: prompts::host_of(failing_uri),
                    failing_uri: failing_uri.to_string(),
                    certificate: cert.clone(),
                    reason: prompts::tls_reason(flags).to_string(),
                });
                true // stop the default error page; the bar owns this
            });
        }
        // Crash containment: a dead web process (segfault, kernel OOM
        // kill) otherwise leaves a silent white window. Surface it in
        // the bar with a reload offer instead.
        {
            let this = self.clone();
            webview.connect_web_process_terminated(move |wv, reason| {
                // discard() terminates the web process on purpose after
                // detaching the view; only report views we still own.
                if this.webview.borrow().as_ref() != Some(wv) {
                    return;
                }
                let reason = match reason {
                    webkit6::WebProcessTerminationReason::Crashed => "crashed",
                    webkit6::WebProcessTerminationReason::ExceededMemoryLimit => {
                        "was killed (out of memory)"
                    }
                    _ => "terminated unexpectedly",
                };
                eprintln!("hwatud: web process for window {} {reason}", this.id);
                this.push_prompt(Prompt::Crash { reason });
            });
        }
        // Popups: window.open / target=_blank. WM-is-the-tab-bar means
        // a popup is just another toplevel. The view must come from
        // open_popup (related_view) or window.opener breaks.
        {
            let this = self.clone();
            webview.connect_create(move |wv, _action| this.open_popup(wv).upcast());
        }
        // Pages may close windows they created (window.close after an
        // OAuth handoff). WebKit only emits this where the web platform
        // allows it, so honoring it unconditionally is safe.
        {
            let this = self.clone();
            webview.connect_close(move |_| this.window.close());
        }
        // Non-displayable responses (Content-Disposition: attachment,
        // MIME types WebKit can't render) become downloads instead of
        // dead ends.
        webview.connect_decide_policy(|_, decision, decision_type| {
            if decision_type != webkit6::PolicyDecisionType::Response {
                return false; // default handling
            }
            let Some(response) = decision.dynamic_cast_ref::<webkit6::ResponsePolicyDecision>()
            else {
                return false;
            };
            if response.is_mime_type_supported() {
                return false;
            }
            decision.download();
            true
        });
        self.overlay.set_child(Some(&webview));
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
        // Never discard a focused window or one awaiting bar input
        // (find in progress, pending permission/TLS prompt).
        if self.window.is_active() || self.bar.is_open() || self.prompts.has_pending() {
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
        // Serialize history to disk, not RAM: the whole point is to
        // give memory back. Write failures degrade to a URL reload.
        let session_file = webview
            .session_state()
            .and_then(|state| state.serialize())
            .and_then(|bytes| {
                let path = discard_dir()?.join(format!("window-{}.session", self.id));
                std::fs::write(&path, bytes).ok()?;
                Some(path)
            });
        self.saved.replace(Some(SavedState {
            session_file,
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
        self.overlay.set_child(Some(&placeholder));
        // Dropping the WebView alone is not enough: WebKit keeps the web
        // process cached for reuse. State is already serialized, so kill
        // the process outright; that is where the RAM comes back — unless
        // a related window (popup ↔ opener) still runs in that process.
        let shared = self
            .process_group
            .borrow_mut()
            .take()
            .is_some_and(|group| Rc::strong_count(&group) > 1);
        if !shared {
            webview.terminate_web_process();
        }
        drop(webview);
    }

    /// Bring a discarded window back to life from the prewarm pool.
    /// pub(crate): automation (eval/navigate) must revive a discarded
    /// window before touching its WebView.
    pub(crate) fn restore(self: &Rc<Self>) {
        if self.webview.borrow().is_some() {
            return;
        }
        let Some(saved) = self.saved.borrow_mut().take() else {
            return;
        };
        let webview = self.daemon.take_webview();
        // Read the session blob back and delete it; a missing/corrupt
        // file (or a state WebKit rejects) falls back to the raw URL.
        if let Some(path) = &saved.session_file {
            if let Ok(bytes) = std::fs::read(path) {
                let state = webkit6::WebViewSessionState::new(&glib::Bytes::from_owned(bytes));
                webview.restore_session_state(&state);
            }
            let _ = std::fs::remove_file(path);
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
                focused: self.window.is_active(),
                suspended: false,
                app_id: self.app_id.clone(),
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
                    focused: false,
                    suspended: true,
                    app_id: self.app_id.clone(),
                }
            }
        }
    }

    pub fn close(&self) {
        self.window.close();
    }

    /// Passive bar message (downloads etc.). No-op if the bar is busy
    /// with an interactive prompt.
    pub fn flash_bar(&self, message: &str, secs: u64) {
        self.bar.flash(message, secs);
    }

    /// The live WebView, if this window is not discarded. Used by the
    /// adblock toggle to re-apply filters across all windows.
    pub fn live_webview(&self) -> Option<webkit6::WebView> {
        self.webview.borrow().clone()
    }

    /// Scroll the page by `pages` half-viewports (negative = up). Runs
    /// in the page's JS world; a discarded window has no page and this
    /// is a no-op.
    fn scroll_page(&self, pages: f64) {
        let Some(webview) = self.live_webview() else {
            return;
        };
        let js = format!("window.scrollBy(0, window.innerHeight * 0.5 * {pages});");
        webview.evaluate_javascript(&js, None, None, gtk::gio::Cancellable::NONE, |_| {});
    }

    // ---- bar & keyboard UX ----------------------------------------

    /// Run one keymap action. Returns Proceed only for actions that
    /// decline (n/N with no committed search fall through to the page).
    fn run_action(self: &Rc<Self>, action: keys::Action) -> glib::Propagation {
        use keys::Action;
        match action {
            Action::Close => self.window.close(),
            Action::UrlOpen => self.bar.open_url(""),
            Action::UrlEdit => self.open_url_bar(),
            Action::Find => self.bar.open_find(false),
            Action::FindBack => self.bar.open_find(true),
            Action::FindNext => return self.find_next(true),
            Action::FindPrev => return self.find_next(false),
            Action::ScrollDown => self.scroll_page(1.0),
            Action::ScrollUp => self.scroll_page(-1.0),
            Action::Back => self.history_go(false),
            Action::Forward => self.history_go(true),
        }
        glib::Propagation::Stop
    }

    /// Back/forward through this window's history. Restores a
    /// discarded window first (its history came back with the blob).
    fn history_go(self: &Rc<Self>, forward: bool) {
        self.restore();
        let Some(webview) = self.live_webview() else {
            return;
        };
        if forward {
            webview.go_forward();
        } else {
            webview.go_back();
        }
    }

    /// Window-level keys. This controller is on the toplevel in the
    /// default (bubble) phase, so it only sees keys the WebView did
    /// not consume: `/` in a page's text box stays in the page, `/`
    /// anywhere else opens find. Modified chords never reach here;
    /// they are handled by the capture controller.
    fn on_window_key(
        self: &Rc<Self>,
        key: gtk::gdk::Key,
        state: gtk::gdk::ModifierType,
    ) -> glib::Propagation {
        match self.bar.mode() {
            BarMode::Hidden | BarMode::Status => {
                match self.daemon.keymap.lookup(keys::Phase::Bubble, key, state) {
                    Some(action) => self.run_action(action),
                    None if key == gtk::gdk::Key::Escape => {
                        self.stop_find();
                        self.bar.close();
                        glib::Propagation::Proceed
                    }
                    None => glib::Propagation::Proceed,
                }
            }
            // Find mode: entry has focus and eats printable keys; we
            // only see what bubbles past it (Escape/Enter handled in
            // wire_bar on the entry itself). Swallow the rest so keys
            // don't leak into the page under the bar.
            BarMode::Find { .. } | BarMode::Url => glib::Propagation::Proceed,
            BarMode::Confirm { tag } => {
                match key {
                    gtk::gdk::Key::y | gtk::gdk::Key::Y => self.answer_confirm(&tag, true),
                    gtk::gdk::Key::n | gtk::gdk::Key::N | gtk::gdk::Key::Escape => {
                        self.answer_confirm(&tag, false)
                    }
                    _ => {}
                }
                glib::Propagation::Stop
            }
        }
    }

    /// Hook the bar's entry: incremental find while typing, Enter
    /// commits (focus returns to page, n/N work), Escape cancels.
    fn wire_bar(self: &Rc<Self>) {
        // Incremental search on every keystroke.
        {
            let this = self.clone();
            self.bar.entry.connect_changed(move |entry| {
                if let BarMode::Find { backwards } = this.bar.mode() {
                    this.run_find(&entry.text(), backwards);
                }
            });
        }
        // Enter: find keeps highlights and returns focus to the page;
        // URL mode navigates.
        {
            let this = self.clone();
            self.bar
                .entry
                .connect_activate(move |entry| match this.bar.mode() {
                    BarMode::Find { .. } => {
                        this.bar.close();
                        this.focus_webview();
                    }
                    BarMode::Url => {
                        let text = entry.text().trim().to_string();
                        this.bar.close();
                        if !text.is_empty() {
                            this.navigate(&text);
                        }
                        this.focus_webview();
                    }
                    _ => {}
                });
        }
        // Escape inside the entry: cancel find/URL entry entirely.
        {
            let this = self.clone();
            let ctrl = gtk::EventControllerKey::new();
            ctrl.connect_key_pressed(move |_, key, _, _| {
                if key == gtk::gdk::Key::Escape {
                    this.stop_find();
                    this.bar.close();
                    this.focus_webview();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            self.bar.entry.add_controller(ctrl);
        }
    }

    /// Open the URL prompt prefilled with the current page's URL.
    fn open_url_bar(self: &Rc<Self>) {
        // A discarded window revives on focus before anyone can type,
        // but restore() explicitly for the WM-rule corner cases.
        self.restore();
        let current = self
            .webview
            .borrow()
            .as_ref()
            .and_then(|wv| wv.uri())
            .map(|u| u.to_string())
            .unwrap_or_default();
        self.bar.open_url(&current);
    }

    /// Navigate this window, normalizing bare hosts the same way the
    /// CLI does (`example.com` -> `https://example.com`).
    fn navigate(self: &Rc<Self>, input: &str) {
        self.restore();
        if let Some(webview) = self.live_webview() {
            webview.load_uri(&crate::ipc_server::normalize_url(input.to_string()));
        }
    }

    fn find_controller(&self) -> Option<webkit6::FindController> {
        self.webview
            .borrow()
            .as_ref()
            .and_then(|wv| wv.find_controller())
    }

    fn run_find(self: &Rc<Self>, text: &str, backwards: bool) {
        let Some(fc) = self.find_controller() else {
            return;
        };
        if text.is_empty() {
            fc.search_finish();
            self.bar.set_status("");
            return;
        }
        let mut opts = webkit6::FindOptions::CASE_INSENSITIVE | webkit6::FindOptions::WRAP_AROUND;
        if backwards {
            opts |= webkit6::FindOptions::BACKWARDS;
        }
        // Wire match-count feedback once per controller instance.
        self.wire_find_signals(&fc);
        fc.count_matches(text, opts.bits(), u32::MAX);
        fc.search(text, opts.bits(), u32::MAX);
    }

    fn wire_find_signals(self: &Rc<Self>, fc: &webkit6::FindController) {
        // Idempotence: tag the controller so signals connect once even
        // though run_find fires per keystroke.
        unsafe {
            if fc.data::<bool>("hwatu-wired").is_some() {
                return;
            }
            fc.set_data("hwatu-wired", true);
        }
        let bar = self.bar.clone();
        fc.connect_counted_matches(move |_, n| {
            bar.set_status(&format!("{n} match{}", if n == 1 { "" } else { "es" }));
        });
        let bar = self.bar.clone();
        fc.connect_failed_to_find_text(move |_| {
            bar.set_status("no matches");
        });
    }

    fn find_next(self: &Rc<Self>, forward: bool) -> glib::Propagation {
        let Some(fc) = self.find_controller() else {
            return glib::Propagation::Proceed;
        };
        // No committed search: let n/N through to the page.
        if fc.search_text().is_none_or(|t| t.is_empty()) {
            return glib::Propagation::Proceed;
        }
        if forward {
            fc.search_next();
        } else {
            fc.search_previous();
        }
        glib::Propagation::Stop
    }

    fn stop_find(&self) {
        if let Some(fc) = self.find_controller() {
            fc.search_finish();
        }
    }

    fn focus_webview(&self) {
        if let Some(wv) = self.webview.borrow().as_ref() {
            wv.grab_focus();
        }
    }

    /// Queue a permission/TLS question; open the bar if it is next.
    fn push_prompt(self: &Rc<Self>, prompt: Prompt) {
        if let Some(question) = self.prompts.push(prompt) {
            self.bar.open_confirm("prompt", &question);
            // Test hook: HWATU_TEST_CONFIRM=allow|deny auto-answers
            // prompts, for headless/scripted verification only.
            if let Ok(answer) = std::env::var("HWATU_TEST_CONFIRM") {
                let this = self.clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || {
                    this.answer_confirm("prompt", answer == "allow");
                });
            }
        }
    }

    /// Resolve the pending y/n prompt, then show the next queued one.
    fn answer_confirm(self: &Rc<Self>, _tag: &str, yes: bool) {
        let next = self
            .prompts
            .answer_front(yes, self.webview.borrow().as_ref());
        match next {
            Some(question) => self.bar.open_confirm("prompt", &question),
            None => {
                self.bar.close();
                self.focus_webview();
            }
        }
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
