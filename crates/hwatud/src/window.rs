// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
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
use crate::launcher;
use crate::prompts::{self, Prompt, Prompts};
use crate::Daemon;
use gtk::prelude::*;
use hwatu_ipc::{OpenMode, WindowInfo};
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

/// How long a focus-promoted window may sit unattended before the
/// auto-demote watchdog checks it (see `try_auto_demote`). Override
/// with HWATU_AUTO_DEMOTE_SECS; 0 disables the watchdog.
fn auto_demote_secs() -> u64 {
    std::env::var("HWATU_AUTO_DEMOTE_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(120)
}

/// Viewport pushed into headless windows, `WIDTHxHEIGHT`. Override
/// with HWATU_HEADLESS_SIZE. Defaults to a common desktop size so
/// responsive pages render their desktop layout instead of tripping
/// tablet breakpoints at 1024px.
fn headless_size() -> (i32, i32) {
    std::env::var("HWATU_HEADLESS_SIZE")
        .ok()
        .and_then(|v| {
            let (w, h) = v.trim().split_once(['x', 'X'])?;
            Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
        })
        .filter(|&(w, h)| w > 0 && h > 0)
        .unwrap_or((1920, 1080))
}

/// Page a bare `hwatu` opens, if the user configured one with
/// HWATU_HOME (any URL, or `about:blank`). Unset means the built-in
/// launcher page with the URL bar pre-opened.
fn home_page() -> Option<String> {
    std::env::var("HWATU_HOME")
        .ok()
        .filter(|v| !v.trim().is_empty())
}

/// Convert an otherwise-unhandled printable key into the first character
/// for the URL/search prompt. Modified chords stay available to the keymap;
/// whitespace stays native so a bare Space still scrolls the page.
fn printable_key_text(key: gtk::gdk::Key, state: gtk::gdk::ModifierType) -> Option<String> {
    let blocked = gtk::gdk::ModifierType::CONTROL_MASK
        | gtk::gdk::ModifierType::ALT_MASK
        | gtk::gdk::ModifierType::META_MASK
        | gtk::gdk::ModifierType::SUPER_MASK
        | gtk::gdk::ModifierType::HYPER_MASK;
    if state.intersects(blocked) {
        return None;
    }
    let ch = key.to_unicode()?;
    if ch.is_control() || ch.is_whitespace() {
        return None;
    }
    Some(ch.to_string())
}

/// Media autoplay policy for every WebView. WebKit's own default is
/// ALLOW_WITHOUT_SOUND, which is why video sites autoplay muted: their
/// unmuted-play probe is rejected, so they fall back to a muted player.
/// hwatu defaults to full allow — sound is expected of a browser one
/// actually watches things in. Override with HWATU_AUTOPLAY=muted (the
/// WebKit default) or =deny (no autoplay at all).
fn autoplay_policy() -> webkit6::AutoplayPolicy {
    // HWATU_BLOCK_AUTOPLAY=1 is the older escape hatch for a
    // WebKitGTK+GStreamer wedge (gst 1.28.5: pages with several
    // lazy-initialized autoplay videos deadlock the web process);
    // scripts still set it, so it stays as an alias for deny.
    if std::env::var_os("HWATU_BLOCK_AUTOPLAY").is_some_and(|v| v == "1") {
        return webkit6::AutoplayPolicy::Deny;
    }
    match std::env::var("HWATU_AUTOPLAY").as_deref() {
        Ok("muted") | Ok("without-sound") => webkit6::AutoplayPolicy::AllowWithoutSound,
        Ok("deny") | Ok("off") => webkit6::AutoplayPolicy::Deny,
        _ => webkit6::AutoplayPolicy::Allow,
    }
}

const DEFAULT_WINDOW_WIDTH: i32 = 1024;
const DEFAULT_WINDOW_HEIGHT: i32 = 768;

/// Request a quarter of the current monitor's width for a newly mapped
/// window. Tiling WMs use this as the initial size hint when deciding how to
/// place a new toplevel, while floating WMs still get a useful desktop-sized
/// window instead of an arbitrary fixed width.
fn default_window_width() -> i32 {
    let Some(display) = gtk::gdk::Display::default() else {
        return DEFAULT_WINDOW_WIDTH;
    };
    let Some(monitor) = display
        .monitors()
        .item(0)
        .and_then(|object| object.downcast::<gtk::gdk::Monitor>().ok())
    else {
        return DEFAULT_WINDOW_WIDTH;
    };

    quarter_width(monitor.geometry().width())
}

fn quarter_width(viewport_width: i32) -> i32 {
    (viewport_width / 4).max(1)
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum RecoveryOverlay {
    Loading,
    Failure,
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
    /// Frozen-frame overlay shown above a restoring WebView so the swap
    /// from placeholder to live page never flashes blank. Dropped when
    /// the restored load commits (or on a timeout failsafe).
    veil: RefCell<Option<gtk::Widget>>,
    /// Human-visible diagnostic overlay for cases that otherwise look
    /// like a featureless gray WebKit surface: crashed processes,
    /// network/load failures, or a page that stays blank while loading.
    recovery: RefCell<Option<(gtk::Widget, RecoveryOverlay)>>,
    discard_timer: RefCell<Option<glib::SourceId>>,
    /// Marker shared between windows whose WebViews share one web
    /// process (popup ↔ opener, via `related_view`). `discard` must
    /// not terminate a process another live window still uses;
    /// `Rc::strong_count > 1` is that test. Cleared on discard: a
    /// restored view comes from the pool and is no longer related.
    process_group: RefCell<Option<Rc<()>>>,
    /// Wayland app_id this window was opened with, echoed by `hwatu list`.
    app_id: Option<String>,
    /// How this window is shown (normal/background/headless). Popups
    /// it spawns inherit it so a headless page can't steal focus. A
    /// `focus` request promotes the window to Normal.
    mode: std::cell::Cell<OpenMode>,
    /// The mode this window had before a `focus` request promoted it
    /// to Normal. Present only while promoted: `unfocus` (explicit or
    /// via the auto-demote watchdog) restores it, so an agent window
    /// surfaced for a CAPTCHA or OAuth click goes back out of the
    /// user's way instead of squatting in the WM forever.
    promoted_from: std::cell::Cell<Option<OpenMode>>,
    /// Watchdog for promoted windows; see [`Self::schedule_auto_demote`].
    demote_timer: RefCell<Option<glib::SourceId>>,
    /// Per-window viewport override (CSS px), set by `hwatu resize`.
    /// None means the headless_size() default. Headless allocation and
    /// ensure_viewport() re-assert whichever is current, so a resized
    /// window keeps its size across navigations.
    viewport: std::cell::Cell<Option<(i32, i32)>>,
    /// URI of a requested-but-not-yet-Started navigation. `is_loading`
    /// is false between `load_uri` and WebKit's LoadEvent::Started, so
    /// `wait_load` needs this to not answer early and let the caller's
    /// next eval be destroyed by the commit. Keyed by URI rather than a
    /// bool: a stale prewarm load's own Started must not clear a real
    /// pending navigation to a different URI.
    nav_pending: RefCell<Option<String>>,
    /// Last navigation target requested on this window (never
    /// cleared, unlike `nav_pending`). Lets stage-aware waits tell a
    /// stale prewarm `about:blank` commit from a genuine navigation
    /// to about:blank.
    nav_target: RefCell<Option<String>>,
    /// True once the current load has Committed: the new document has
    /// replaced the old one. Together with `nav_pending` this lets
    /// stage-aware waits (`wait-load --until committed|dom`) know
    /// whether an eval would target the requested document or a stale
    /// one. Starts true: an idle window's document is its document.
    load_committed: std::cell::Cell<bool>,
    /// Baseline for `hwatu snapshot --diff`: the previous diff
    /// snapshot of this window in normalized form. `None` until the
    /// first diff snapshot and after every cross-document navigation
    /// (a new document is a new page; diffing across it would report
    /// the whole world as changed anyway, and the agent is told
    /// explicitly via `baseline_established` instead).
    pub snapshot_baseline: RefCell<Option<Vec<crate::snapdiff::Node>>>,
    /// Console/error/network capture for `hwatu console`. Outlives
    /// discards: the page's state dies, what it logged did happen.
    pub console: crate::console::Buffer,
    /// Structured request log for `hwatu net`: every resource load
    /// (method, url, status, type, timing), not just failures.
    /// Outlives discards for the same reason the console buffer does.
    pub net: crate::net::Buffer,
    /// Command-palette state while the bar is in Palette mode: the
    /// current ranked matches and which one is highlighted. Cleared on
    /// close so a reopened palette starts fresh.
    palette: RefCell<Option<PaletteState>>,
}

/// See [`BrowserWindow::palette`].
struct PaletteState {
    matches: Vec<keys::Action>,
    selected: usize,
}

/// `HWATU_WEBKIT_FEATURES=Ident:on,Other:off`: escape hatch for odd
/// hardware. hwatu used to force-enable the async/threaded scrolling
/// features here, but forcing them breaks wheel scrolling outright on
/// some driver stacks (notably NVIDIA + Wayland), so engine defaults
/// now rule and this env var is the only way to flip features.
fn feature_overrides() -> Vec<(String, bool)> {
    std::env::var("HWATU_WEBKIT_FEATURES")
        .map(|raw| parse_feature_overrides(&raw))
        .unwrap_or_default()
}

/// Features hwatu flips away from the engine default. Env overrides
/// (`HWATU_WEBKIT_FEATURES`) win over these, so
/// `PropagateDamagingInformation:on` re-enables it for testing.
///
/// `PropagateDamagingInformation` (default on since WebKitGTK 2.52)
/// makes the compositor upload only damaged regions. On some stacks
/// the damage rects are wrong and stale/uninitialized buffer rows show
/// through as horizontal black bars: observed here on fractional-scale
/// Wayland (niri at 1.25) after animations moving fixed-position
/// layers, and on Intel+NVIDIA hybrid laptops while scrolling (WebKit
/// bugs 305560/305758 landed fixes upstream after 2.52 branched).
/// Chromium-family browsers don't share this path, which is why the
/// same pages look fine elsewhere. Trade a little GPU bandwidth for
/// correct pixels until the fixes ship in a stable WebKitGTK.
const BASELINE_FEATURE_OVERRIDES: &[(&str, bool)] = &[("PropagateDamagingInformation", false)];

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
/// a fallback; all engine knobs live here, never on the spawn path.
pub fn build_webview() -> webkit6::WebView {
    // website-policies is construct-only, hence the builder. Autoplay
    // defaults to full allow (with sound); see autoplay_policy().
    let policies = webkit6::WebsitePolicies::builder()
        .autoplay(autoplay_policy())
        .build();
    let view = webkit6::WebView::builder()
        .website_policies(&policies)
        .build();
    apply_view_settings(&view);
    crate::console::wire_view(&view);
    crate::clock::wire_view(&view);
    crate::smoothwheel::wire_view(&view);
    crate::focusshield::wire_view(&view);
    crate::blurshield::wire_view(&view);
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

        // Baseline feature flips (see BASELINE_FEATURE_OVERRIDES),
        // then env overrides on top so HWATU_WEBKIT_FEATURES can
        // re-enable anything hwatu turns off by default.
        let overrides = feature_overrides();
        if let Some(features) = webkit6::Settings::all_features() {
            for i in 0..features.length() {
                let Some(feature) = features.get(i) else {
                    continue;
                };
                let ident = feature.identifier().unwrap_or_default();
                if let Some((_, on)) = BASELINE_FEATURE_OVERRIDES
                    .iter()
                    .find(|(name, _)| *name == ident)
                {
                    settings.set_feature_enabled(&feature, *on);
                }
                if let Some((_, on)) = overrides.iter().find(|(name, _)| *name == ident) {
                    settings.set_feature_enabled(&feature, *on);
                }
            }
        }
    }
}

impl BrowserWindow {
    pub fn open(
        daemon: &Rc<Daemon>,
        url: Option<String>,
        app_id: Option<String>,
        mode: OpenMode,
    ) -> WindowInfo {
        // On Wayland the compositor decides who gets focus, and most
        // tilers focus new windows. A background window therefore gets
        // a predictable default app_id so one WM rule can opt it out
        // (niri: `match app-id="hwatu-background"` + `open-focused
        // false`; Hyprland: `windowrule = noinitialfocus, class:...`).
        let app_id = app_id
            .or_else(|| (mode == OpenMode::Background).then(|| "hwatu-background".to_string()));
        let webview = daemon.take_webview();
        let this = Self::build(daemon, webview.clone(), app_id.clone(), mode);
        // No URL and no configured home page: show the launcher (the
        // keybind cheat sheet) with the URL bar already open, so a
        // bare `hwatu` is "type where you want to go".
        let target = match url.or_else(home_page) {
            Some(url) => {
                this.mark_nav_pending(&url);
                webview.load_uri(&url);
                url
            }
            None => {
                let uri = launcher::deal_uri(daemon.take_deal());
                this.mark_nav_pending(&uri);
                webview.load_uri(&uri);
                if mode == OpenMode::Normal {
                    this.bar.open_url("");
                }
                uri
            }
        };
        this.show();
        WindowInfo {
            id: this.id,
            url: target,
            title: String::new(),
            focused: false,
            suspended: false,
            app_id,
            mode,
        }
    }

    /// Map the window according to its open mode. `present` asks the
    /// compositor for focus; `set_visible` maps without an activation
    /// request, so the user's focus stays put; headless never maps.
    fn show(self: &Rc<Self>) {
        match self.mode.get() {
            OpenMode::Normal => self.window.present(),
            OpenMode::Background => {
                // Realize (create the xdg_toplevel) before mapping so
                // the app_id is already set when the compositor first
                // sees the window; initial-focus window rules match
                // against it. The post-map idle in build() is a no-op
                // repeat for this path.
                gtk::prelude::WidgetExt::realize(&self.window);
                if let Some(app_id) = &self.app_id {
                    set_wayland_app_id(&self.window, app_id);
                }
                self.window.set_visible(true);
            }
            OpenMode::Headless => {
                // No map: the WM never sees this window. But WebKit lays
                // pages out at the widget's allocated size, and an
                // unallocated widget is 0x0 (pages collapse, snapshots
                // fail). Realizing the toplevel creates its GDK surface
                // without mapping, and a manual allocation pushes a real
                // viewport into the web process.
                self.allocate_viewport();
            }
        }
    }

    /// Set the window's viewport allocation (logical px). The verify
    /// layer converts CSS px to logical px using the page's own
    /// devicePixelRatio (fractional scale is not readable on unmapped
    /// surfaces), so this just stores and applies the allocation. For
    /// matrix verification: responsive pages are only "verified" at
    /// the widths actually sampled, so agents step a warm window
    /// through widths instead of paying a context spawn per size.
    pub fn resize_viewport(self: &Rc<Self>, w: i32, h: i32) {
        self.viewport.set(Some((w, h)));
        match self.mode.get() {
            OpenMode::Headless => self.allocate_viewport(),
            _ => self.window.set_default_size(w, h),
        }
    }

    /// Drop any per-window viewport override, returning the window to
    /// the headless_size() default. A multi-viewport check sweep calls
    /// this before parking its window so the next (plain) check does
    /// not inherit the sweep's last size.
    pub fn reset_viewport(self: &Rc<Self>) {
        if self.viewport.take().is_some() && self.mode.get() == OpenMode::Headless {
            self.allocate_viewport();
        }
    }

    /// (Re-)allocate the headless toplevel at the current viewport.
    fn allocate_viewport(self: &Rc<Self>) {
        gtk::prelude::WidgetExt::realize(&self.window);
        let (w, h) = self.viewport.get().unwrap_or_else(headless_size);
        self.window.allocate(w, h, -1, None::<gtk::gsk::Transform>);
    }

    /// Open a window for a popup requested by the page (`window.open`,
    /// `target=_blank`). The new WebView must be built with
    /// `related_view` so it shares the opener's web process;
    /// `window.opener` and postMessage (OAuth flows) depend on it. The
    /// prewarmed pool can't serve this, so the view is built here.
    /// WebKit drives the navigation itself; loading anything manually
    /// would break the popup contract. The window is presented on
    /// ready-to-show, once the engine has applied window features.
    fn open_popup(self: &Rc<Self>, related: &webkit6::WebView) -> webkit6::WebView {
        let popup_policies = webkit6::WebsitePolicies::builder()
            .autoplay(autoplay_policy())
            .build();
        let webview = webkit6::WebView::builder()
            .related_view(related)
            .website_policies(&popup_policies)
            .build();
        apply_view_settings(&webview);
        crate::console::wire_view(&webview);
        crate::clock::wire_view(&webview);
        crate::smoothwheel::wire_view(&webview);
        crate::focusshield::wire_view(&webview);
        self.daemon.adblock.apply_to(&webview);
        let popup = Self::build(
            &self.daemon,
            webview.clone(),
            self.app_id.clone(),
            self.mode.get(),
        );
        // Mark both windows as sharing a web process so neither
        // discard() terminates it while the other still needs it.
        let group = self
            .process_group
            .borrow_mut()
            .get_or_insert_with(|| Rc::new(()))
            .clone();
        popup.process_group.replace(Some(group));
        {
            let popup = popup.clone();
            webview.connect_ready_to_show(move |_| popup.show());
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
        mode: OpenMode,
    ) -> Rc<Self> {
        let id = daemon.alloc_id();

        let window = gtk::Window::builder()
            .application(&daemon.app)
            .default_width(default_window_width())
            .default_height(DEFAULT_WINDOW_HEIGHT)
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
                // Synchronous first: if the xdg_toplevel already exists
                // here, the app_id rides the initial commit and the
                // compositor's open-time window rules (initial focus,
                // workspace) match it. The idle repeat covers GTK
                // versions where the role appears later in the map.
                set_wayland_app_id(win, &app_id);
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
            veil: RefCell::new(None),
            recovery: RefCell::new(None),
            discard_timer: RefCell::new(None),
            process_group: RefCell::new(None),
            app_id,
            mode: std::cell::Cell::new(mode),
            promoted_from: std::cell::Cell::new(None),
            demote_timer: RefCell::new(None),
            viewport: std::cell::Cell::new(None),
            nav_pending: RefCell::new(None),
            nav_target: RefCell::new(None),
            load_committed: std::cell::Cell::new(true),
            snapshot_baseline: RefCell::new(None),
            console: crate::console::Buffer::default(),
            net: crate::net::Buffer::default(),
            palette: RefCell::new(None),
        });

        this.attach_webview(webview);
        this.wire_bar();

        // Push-IPC fan-out: console captures become `console` events
        // with this window's id, and the window's own birth is a
        // `window` event. Installed once; the Buffer outlives WebView
        // swaps (discard/restore), so this survives them.
        {
            let daemon = daemon.clone();
            let win_id = id;
            this.console.set_hook(move |entry| {
                daemon.events.emit(
                    "console",
                    Some(win_id),
                    serde_json::to_value(entry).unwrap_or_default(),
                );
            });
        }
        daemon.events.emit(
            "window",
            Some(id),
            serde_json::json!({ "state": "opened", "mode": mode }),
        );

        // Ctrl+w (or ctrl+q) closes the window; the daemon (and engine) stay warm.
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
                // While the bar's entry owns focus (find/URL/palette
                // typing), global chords stay out of the way: Ctrl+o in
                // a URL prompt must not navigate history under the bar.
                let entry_open = matches!(
                    this2.bar.mode(),
                    BarMode::Find { .. } | BarMode::Url | BarMode::Palette
                );
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
                // The GTK toplevel dying does not kill the web process:
                // WebKit caches it for reuse, and a cached process keeps
                // running its page — an autoplaying video's audio audibly
                // outlives the window (and the agent session that opened
                // it). Kill it here, on the same shared-process guard as
                // the discard path.
                this.terminate_web_process_unless_shared();
                // A discarded window closed for good: its blob is dead.
                if let Some(saved) = this.saved.borrow_mut().take() {
                    if let Some(path) = saved.session_file {
                        let _ = std::fs::remove_file(path);
                    }
                }
                daemon.windows.borrow_mut().remove(&id);
                daemon.schedule_session_save();
                daemon
                    .events
                    .emit("window", Some(id), serde_json::json!({ "state": "closed" }));
                glib::Propagation::Proceed
            });
        }

        daemon.windows.borrow_mut().insert(id, this.clone());
        daemon.schedule_session_save();
        this
    }

    /// Put a WebView into the window and wire its signals.
    fn attach_webview(self: &Rc<Self>, webview: webkit6::WebView) {
        crate::console::attach(&self.console, &webview);
        crate::net::attach(&self.net, &webview);
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
        // A gray WebKit surface during a slow or wedged load used to look
        // like hwatu itself had broken. Surface a low-priority overlay if
        // the page is still blank after a short grace period, and clear it
        // on commit/finish. Real load failures and crashes get stronger
        // overlays below.
        {
            let this = self.clone();
            webview.connect_load_changed(move |wv, event| match event {
                webkit6::LoadEvent::Started => {
                    this.note_load_engaged(wv);
                    this.load_committed.set(false);
                    this.clear_recovery_overlay();
                    this.emit_load(wv, "started");
                    let this = this.clone();
                    let wv = wv.clone();
                    glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || {
                        if this.webview.borrow().as_ref() != Some(&wv) || !wv.is_loading() {
                            return;
                        }
                        let title = wv.title().unwrap_or_default();
                        if title.is_empty() {
                            this.show_recovery_overlay(
                                "Still loading",
                                "If this stays gray, press Ctrl+r to reload or Ctrl+l to open a URL.",
                                RecoveryOverlay::Loading,
                            );
                        }
                    });
                }
                webkit6::LoadEvent::Committed => {
                    this.note_load_engaged(wv);
                    this.load_committed.set(true);
                    // A new document invalidates the snapshot-diff
                    // baseline: the next `snapshot --diff` starts over.
                    this.snapshot_baseline.replace(None);
                    this.clear_recovery_overlay();
                    this.emit_load(wv, "committed");
                }
                webkit6::LoadEvent::Finished => {
                    this.clear_loading_recovery_overlay();
                    this.emit_load(wv, "finished");
                }
                _ => {}
            });
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
                this.show_recovery_overlay(
                    "TLS error",
                    &format!(
                        "{} ({})\nPress y in the bar to proceed for this session, or n/Esc to stop.",
                        prompts::host_of(failing_uri),
                        prompts::tls_reason(flags)
                    ),
                    RecoveryOverlay::Failure,
                );
                this.push_prompt(Prompt::Tls {
                    host: prompts::host_of(failing_uri),
                    failing_uri: failing_uri.to_string(),
                    certificate: cert.clone(),
                    reason: prompts::tls_reason(flags).to_string(),
                });
                true // stop the default error page; the bar owns this
            });
        }
        // Non-TLS load failures: replace mystery gray with a visible
        // explanation and recovery keys. Returning false keeps WebKit's
        // normal error handling if it has one; the overlay is just hwatu's
        // persistent diagnostic layer.
        {
            let this = self.clone();
            webview.connect_load_failed(move |_, _, failing_uri, error| {
                this.daemon.events.emit(
                    "load",
                    Some(this.id),
                    serde_json::json!({
                        "state": "failed",
                        "url": failing_uri,
                        "error": error.to_string(),
                    }),
                );
                this.show_recovery_overlay(
                    "Page failed to load",
                    &format!(
                        "{failing_uri}\n{error}\nPress Ctrl+r to retry or Ctrl+l to open a new URL."
                    ),
                    RecoveryOverlay::Failure,
                );
                false
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
                this.show_recovery_overlay(
                    "Page crashed",
                    &format!("The web process {reason}. Press y in the bar to reload, or Ctrl+l to open a URL."),
                    RecoveryOverlay::Failure,
                );
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
        // dead ends. Main-document responses also pass through the
        // per-site UA switcher (mobile UI for reels-style sites),
        // which may restart the load under the right user-agent.
        webview.connect_decide_policy(|wv, decision, decision_type| {
            if decision_type != webkit6::PolicyDecisionType::Response {
                return false; // default handling
            }
            let Some(response) = decision.dynamic_cast_ref::<webkit6::ResponsePolicyDecision>()
            else {
                return false;
            };
            if crate::siteua::handle_response_decision(wv, response) {
                return true;
            }
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

    /// True while any of the discard guards forbid tearing down the
    /// WebView: focused window, open bar, pending prompt.
    fn discard_blocked(&self) -> bool {
        self.window.is_active() || self.bar.is_open() || self.prompts.has_pending()
    }

    /// Serialize state, destroy the WebView, show a placeholder.
    ///
    /// Two-phase: first snapshot the visible page into a texture (async,
    /// the web process must still be alive to paint it), then tear down
    /// in the callback. The placeholder shows the frozen frame, so a
    /// suspended window is indistinguishable from a live one at a
    /// glance; a plain title label is the fallback if snapshotting
    /// fails.
    fn discard(self: &Rc<Self>) {
        if self.discard_blocked() {
            return;
        }
        let Some(webview) = self.webview.borrow().clone() else {
            return;
        };
        if webview.is_loading() || webview.is_playing_audio() {
            // Try again later rather than losing an in-flight load or
            // cutting off media playing in the background (music, a
            // reel left running in another window).
            self.schedule_discard();
            return;
        }
        let this = self.clone();
        webview.snapshot(
            webkit6::SnapshotRegion::Visible,
            webkit6::SnapshotOptions::NONE,
            gtk::gio::Cancellable::NONE,
            move |result| this.finish_discard(result.ok()),
        );
    }

    /// Second phase of [`Self::discard`]: runs after the snapshot resolves.
    /// The guards are re-checked because focus, prompts, or navigation
    /// may have changed during the async gap.
    fn finish_discard(self: &Rc<Self>, snapshot: Option<gtk::gdk::Texture>) {
        if self.discard_blocked() {
            return;
        }
        let Some(webview) = self.webview.borrow_mut().take() else {
            return;
        };
        if webview.is_loading() || webview.is_playing_audio() {
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

        // Frozen frame if the snapshot succeeded; the page keeps its
        // last-painted look while suspended. Title label as fallback.
        let placeholder: gtk::Widget = match snapshot {
            Some(texture) => gtk::Picture::builder()
                .paintable(&texture)
                .content_fit(gtk::ContentFit::Cover)
                .build()
                .upcast(),
            None => gtk::Label::builder()
                .label(if title.is_empty() {
                    "(suspended)"
                } else {
                    &title
                })
                .build()
                .upcast(),
        };
        self.overlay.set_child(Some(&placeholder));
        // Dropping the WebView alone is not enough: WebKit keeps the web
        // process cached for reuse. State is already serialized, so kill
        // the process outright; that is where the RAM comes back, unless
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
        // Speed of *perceived* restore: re-float the frozen frame (the
        // outgoing placeholder) above the fresh WebView and lift it on
        // the first paint-worthy load event. Without this the window
        // flashes blank while the page re-commits.
        if let Some(placeholder) = self.overlay.child() {
            self.drop_veil(); // stale veil from a prior restore, if any
                              // A widget cannot be parented twice: detach it as the main
                              // child (attach_webview fills that slot) before floating it.
            self.overlay.set_child(gtk::Widget::NONE);
            self.overlay.add_overlay(&placeholder);
            self.veil.replace(Some(placeholder));
            let this = self.clone();
            webview.connect_load_changed(move |_, event| {
                // Committed fires when the first bytes render;
                // Finished covers same-document restores that skip it.
                if matches!(
                    event,
                    webkit6::LoadEvent::Committed | webkit6::LoadEvent::Finished
                ) {
                    this.drop_veil();
                }
            });
            // Failsafe: a hung load must not leave a stale frame
            // covering a live, interactive page.
            let this = self.clone();
            glib::timeout_add_local_once(std::time::Duration::from_secs(5), move || {
                this.drop_veil();
            });
        }
        self.attach_webview(webview);
    }

    /// Remove the frozen-frame overlay left by [`Self::restore`], if present.
    fn drop_veil(&self) {
        if let Some(veil) = self.veil.borrow_mut().take() {
            self.overlay.remove_overlay(&veil);
        }
    }

    fn show_recovery_overlay(&self, title: &str, detail: &str, kind: RecoveryOverlay) {
        self.clear_recovery_overlay();
        let panel = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .css_classes(["hwatu-recovery"])
            .build();
        panel.append(
            &gtk::Label::builder()
                .label(title)
                .css_classes(["title"])
                .build(),
        );
        panel.append(
            &gtk::Label::builder()
                .label(detail)
                .wrap(true)
                .max_width_chars(72)
                .justify(gtk::Justification::Center)
                .css_classes(["detail"])
                .build(),
        );
        panel.append(
            &gtk::Label::builder()
                .label("Recovery keys: Ctrl+r reload · Ctrl+l edit URL · Ctrl+w close")
                .css_classes(["hint"])
                .build(),
        );

        let widget: gtk::Widget = panel.upcast();
        self.overlay.add_overlay(&widget);
        self.recovery.replace(Some((widget, kind)));
    }

    fn clear_recovery_overlay(&self) {
        if let Some((widget, _)) = self.recovery.borrow_mut().take() {
            self.overlay.remove_overlay(&widget);
        }
    }

    fn clear_loading_recovery_overlay(&self) {
        let clear = self
            .recovery
            .borrow()
            .as_ref()
            .is_some_and(|(_, kind)| *kind == RecoveryOverlay::Loading);
        if clear {
            self.clear_recovery_overlay();
        }
    }

    /// Raise and focus, promoting background/headless windows to
    /// normal: an explicit focus request means "show me this window".
    /// The prior mode is remembered so `unfocus` (or the auto-demote
    /// watchdog) can put the window back out of the user's way, and a
    /// watchdog is armed: a promoted window that no longer shows any
    /// need for a human (not focused, no bar prompt, no CAPTCHA)
    /// demotes itself instead of squatting in the WM forever.
    pub fn present(self: &Rc<Self>) {
        let prev = self.mode.get();
        if prev != OpenMode::Normal && self.promoted_from.get().is_none() {
            self.promoted_from.set(Some(prev));
        }
        self.mode.set(OpenMode::Normal);
        self.window.present();
        self.schedule_auto_demote();
    }

    /// The inverse of [`Self::present`]: unmap the window and restore
    /// the mode it had before promotion (headless if it was never
    /// promoted, hiding is the caller's evident intent).
    pub fn unfocus(self: &Rc<Self>) {
        self.cancel_demote_timer();
        let restored = self.promoted_from.take().unwrap_or(OpenMode::Headless);
        self.mode.set(restored);
        match restored {
            OpenMode::Background => {
                // Re-map without an activation request: visible in the
                // layout, focus goes back to wherever the WM sends it.
                self.window.set_visible(false);
                self.window.set_visible(true);
            }
            // Normal can't be restored-to (promotion only records
            // non-Normal modes); treat it like headless for safety.
            OpenMode::Headless | OpenMode::Normal => {
                self.window.set_visible(false);
                // Unmapping drops the allocation; push the offscreen
                // viewport back so the page keeps rendering.
                self.allocate_viewport();
            }
        }
    }

    /// Arm (or re-arm) the promoted-window watchdog. Windows are only
    /// ever visible on the user's desktop for one of two reasons: a
    /// human opened them, or an agent surfaced them for human input.
    /// For the second kind, "needs a human" is checkable, so check it:
    /// every `HWATU_AUTO_DEMOTE_SECS` (default 120, 0 disables) a
    /// promoted window verifies that it is focused, has a pending
    /// prompt, or shows a CAPTCHA/anti-bot challenge; failing all
    /// three it returns to its pre-promotion mode.
    fn schedule_auto_demote(self: &Rc<Self>) {
        let secs = auto_demote_secs();
        if secs == 0 || self.promoted_from.get().is_none() {
            return;
        }
        self.cancel_demote_timer();
        let this = self.clone();
        let source =
            glib::timeout_add_local_once(std::time::Duration::from_secs(secs), move || {
                this.demote_timer.replace(None);
                this.try_auto_demote();
            });
        self.demote_timer.replace(Some(source));
    }

    fn cancel_demote_timer(&self) {
        if let Some(source) = self.demote_timer.borrow_mut().take() {
            source.remove();
        }
    }

    /// Watchdog body: demote the window unless something still needs a
    /// human. GTK-visible engagement (focus, bar, prompts) is checked
    /// synchronously; page-level challenges need an async eval, and the
    /// demote only happens when that eval reports the page clear.
    fn try_auto_demote(self: &Rc<Self>) {
        if self.promoted_from.get().is_none() {
            return;
        }
        if self.window.is_active() || self.bar.is_open() || self.prompts.has_pending() {
            self.schedule_auto_demote();
            return;
        }
        let Some(webview) = self.live_webview() else {
            // Discarded while promoted: nothing on screen needs a human.
            self.unfocus();
            return;
        };
        let js = format!(
            "{}\ndetectHwatuChallenge().status",
            crate::automation::challenge_detector_js()
        );
        let this = self.clone();
        webview.evaluate_javascript(
            &js,
            None,
            None,
            gtk::gio::Cancellable::NONE,
            move |result| {
                let challenged = result
                    .ok()
                    .map(|v| v.to_str() == "challenge")
                    .unwrap_or(false);
                // Re-check engagement: the user may have focused the
                // window during the async gap.
                if challenged || this.window.is_active() || this.prompts.has_pending() {
                    this.schedule_auto_demote();
                } else {
                    this.unfocus();
                }
            },
        );
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
                mode: self.mode.get(),
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
                    mode: self.mode.get(),
                }
            }
        }
    }

    pub fn close(&self) {
        self.window.close();
    }

    /// Kill this window's web process unless a related window (popup ↔
    /// opener) still runs in it. Closing a window only destroys the GTK
    /// toplevel; WebKit keeps the web process cached for reuse, and a
    /// cached process keeps running its page — an autoplaying video's
    /// audio audibly outlives the window (and the agent session that
    /// opened it). Same rationale as the discard path in
    /// [`Self::finish_discard`].
    fn terminate_web_process_unless_shared(&self) {
        let shared = self
            .process_group
            .borrow_mut()
            .take()
            .is_some_and(|group| Rc::strong_count(&group) > 1);
        if shared {
            return;
        }
        if let Some(webview) = self.webview.borrow().as_ref() {
            webview.terminate_web_process();
        }
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

    /// Re-assert the offscreen viewport of a headless window. The
    /// manual allocation from `show()` is not sticky: any relayout GTK
    /// runs later (a navigation changing the page's size requests is
    /// enough) re-allocates the unmapped toplevel to 0x0, collapsing
    /// the page layout and breaking snapshots. Automation calls this
    /// before touching the view, so headless windows self-heal.
    pub(crate) fn ensure_viewport(self: &Rc<Self>) {
        if self.mode.get() != OpenMode::Headless {
            return;
        }
        // Judge by the WebView's own allocation, not the toplevel's:
        // GTK can relayout children of an unmapped window to 0x0 while
        // the toplevel still reports its old size.
        let collapsed = self
            .webview
            .borrow()
            .as_ref()
            .map(|v| v.width() <= 0 || v.height() <= 0)
            .unwrap_or(false);
        if collapsed {
            self.allocate_viewport();
        }
    }

    /// Fan a load-lifecycle event out to push-IPC subscribers.
    fn emit_load(&self, wv: &webkit6::WebView, state: &str) {
        self.daemon.events.emit(
            "load",
            Some(self.id),
            serde_json::json!({
                "state": state,
                "url": wv.uri().map(|u| u.to_string()).unwrap_or_default(),
            }),
        );
    }

    /// Mark that a navigation to `uri` was just requested on this
    /// window (see the `nav_pending` field). Automation calls this
    /// around its own `load_uri` so `wait_load` cannot answer in the
    /// request gap.
    pub(crate) fn mark_nav_pending(&self, uri: &str) {
        self.nav_pending.replace(Some(uri.to_string()));
        self.nav_target.replace(Some(uri.to_string()));
        self.load_committed.set(false);
    }

    /// True while a requested navigation has not yet Started.
    pub(crate) fn nav_pending(&self) -> bool {
        self.nav_pending.borrow().is_some()
    }

    /// The most recently requested navigation target, if any.
    pub(crate) fn nav_target(&self) -> Option<String> {
        self.nav_target.borrow().clone()
    }

    /// True once the current (or last) load's document has Committed.
    /// False between a navigation request and its commit. A stale
    /// prewarm load's commit can set this while a real navigation is
    /// still pending, so callers must also check [`Self::nav_pending`].
    pub(crate) fn load_committed(&self) -> bool {
        self.load_committed.get()
    }

    /// A load Started/Committed on this window: clear the pending
    /// marker unless the event belongs to a stale prewarm load. The
    /// pool deep-warms views with about:blank, and adopting one
    /// mid-warm lets that load's events fire after the real navigation
    /// was requested; treating them as "the navigation engaged" made
    /// wait_load answer before the real load began. The stale load is
    /// only ever about:blank (or a not-yet-set URI), so those never
    /// clear a pending navigation to a real URL.
    fn note_load_engaged(&self, wv: &webkit6::WebView) {
        let uri = wv.uri().map(|u| u.to_string()).unwrap_or_default();
        let stale = (uri.is_empty() || uri == "about:blank")
            && self
                .nav_pending
                .borrow()
                .as_ref()
                .is_some_and(|want| want != &uri && want != "about:blank");
        if !stale {
            self.nav_pending.replace(None);
        }
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
    /// decline (repeat-search keys with no committed search fall through to the page).
    fn run_action(self: &Rc<Self>, action: keys::Action) -> glib::Propagation {
        use keys::Action;
        match action {
            Action::Close => self.window.close(),
            Action::NewWindow => {
                Self::open(&self.daemon, None, None, OpenMode::Normal);
            }
            Action::UrlOpen => self.bar.open_url(""),
            Action::UrlEdit => self.open_url_bar(),
            Action::YankUrl => self.yank_url(),
            Action::Find => self.bar.open_find(false),
            Action::FindBack => self.bar.open_find(true),
            Action::FindNext => return self.find_next(true),
            Action::FindPrev => return self.find_next(false),
            Action::ScrollDown => self.scroll_page(1.0),
            Action::ScrollUp => self.scroll_page(-1.0),
            Action::Back => self.history_go(false),
            Action::Forward => self.history_go(true),
            Action::Reload => self.reload(false),
            Action::HardReload => self.reload(true),
            Action::ZoomIn => self.zoom_by(1.1),
            Action::ZoomOut => self.zoom_by(1.0 / 1.1),
            Action::ZoomReset => self.zoom_reset(),
            Action::Fullscreen => {
                if self.window.is_fullscreen() {
                    self.window.unfullscreen();
                } else {
                    self.window.fullscreen();
                }
            }
            Action::CommandPalette => self.open_palette(),
        }
        glib::Propagation::Stop
    }

    /// Copy the current page URL to the desktop clipboard.
    fn yank_url(self: &Rc<Self>) {
        self.restore();
        let Some(url) = self
            .live_webview()
            .and_then(|webview| webview.uri())
            .map(|uri| uri.to_string())
            .filter(|url| !url.is_empty())
        else {
            self.flash_bar("no page URL to copy", 2);
            return;
        };
        let Some(display) = gtk::gdk::Display::default() else {
            self.flash_bar("clipboard unavailable", 2);
            return;
        };
        display.clipboard().set_text(&url);
        self.flash_bar("URL copied", 2);
    }

    /// Reload the current page, optionally bypassing the cache
    /// (ctrl+shift+r, the "the CSS is stale" reflex). Restores a
    /// discarded window first (restore already brings the page back at
    /// its saved state, so this is only a fresh restore in that case).
    fn reload(self: &Rc<Self>, bypass_cache: bool) {
        self.restore();
        let Some(webview) = self.live_webview() else {
            return;
        };
        if bypass_cache {
            webview.reload_bypass_cache();
        } else {
            webview.reload();
        }
    }

    /// Multiply the page zoom by `factor`, clamped to a sane range.
    /// Zoom is per-window state on the WebView, like other browsers.
    fn zoom_by(self: &Rc<Self>, factor: f64) {
        let Some(webview) = self.live_webview() else {
            return;
        };
        let level = (webview.zoom_level() * factor).clamp(0.25, 5.0);
        webview.set_zoom_level(level);
        self.flash_bar(&format!("zoom {:.0}%", level * 100.0), 1);
    }

    /// Back to 100% (ctrl+0).
    fn zoom_reset(self: &Rc<Self>) {
        let Some(webview) = self.live_webview() else {
            return;
        };
        webview.set_zoom_level(1.0);
        self.flash_bar("zoom 100%", 1);
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
    /// not consume. An otherwise-unhandled printable key opens the
    /// current-URL prompt with that character as its first input.
    /// Modified chords are handled by the capture controller.
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
                        if self.close_if_bare_launcher() {
                            return glib::Propagation::Stop;
                        }
                        self.stop_find();
                        self.bar.close();
                        glib::Propagation::Proceed
                    }
                    None => {
                        if let Some(seed) = printable_key_text(key, state) {
                            self.open_url_with_seed(&seed);
                            glib::Propagation::Stop
                        } else {
                            glib::Propagation::Proceed
                        }
                    }
                }
            }
            // Find mode: entry has focus and eats printable keys; we
            // only see what bubbles past it (Escape/Enter handled in
            // wire_bar on the entry itself). Swallow the rest so keys
            // don't leak into the page under the bar.
            BarMode::Find { .. } | BarMode::Url | BarMode::Palette => glib::Propagation::Proceed,
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
    /// commits (focus returns to page, repeat-search keys work), Escape cancels.
    fn wire_bar(self: &Rc<Self>) {
        // Incremental search on every keystroke; palette re-ranks on
        // every keystroke the same way.
        {
            let this = self.clone();
            self.bar
                .entry
                .connect_changed(move |entry| match this.bar.mode() {
                    BarMode::Find { backwards } => this.run_find(&entry.text(), backwards),
                    BarMode::Palette => this.refresh_palette(&entry.text()),
                    _ => {}
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
                    BarMode::Palette => this.run_palette_selected(),
                    _ => {}
                });
        }
        // Escape inside the entry: cancel find/URL entry entirely.
        // On a bare launcher window, cancelling means the window
        // itself was a mis-fire: close it. Up/Down while the palette
        // is open move its highlight instead of the entry cursor.
        {
            let this = self.clone();
            let ctrl = gtk::EventControllerKey::new();
            ctrl.connect_key_pressed(move |_, key, _, _| {
                if key == gtk::gdk::Key::Escape {
                    if this.close_if_bare_launcher() {
                        return glib::Propagation::Stop;
                    }
                    this.palette.replace(None);
                    this.stop_find();
                    this.bar.close();
                    this.focus_webview();
                    return glib::Propagation::Stop;
                }
                if this.bar.mode() == BarMode::Palette {
                    match key {
                        gtk::gdk::Key::Down => {
                            this.palette_move(1);
                            return glib::Propagation::Stop;
                        }
                        gtk::gdk::Key::Up => {
                            this.palette_move(-1);
                            return glib::Propagation::Stop;
                        }
                        _ => {}
                    }
                }
                glib::Propagation::Proceed
            });
            self.bar.entry.add_controller(ctrl);
        }
    }

    // ---- command palette -------------------------------------------

    /// How many ranked matches the palette shows at once.
    const PALETTE_ROWS: usize = 8;

    /// Open the palette over every keymap action.
    fn open_palette(self: &Rc<Self>) {
        self.bar.open_palette();
        self.refresh_palette("");
    }

    /// Re-rank against `query`, reset the highlight, redraw.
    fn refresh_palette(self: &Rc<Self>, query: &str) {
        let items = crate::palette::items(&self.daemon.keymap);
        let ranked = crate::palette::filter(&items, query);
        let rows: Vec<(String, String)> = ranked
            .iter()
            .take(Self::PALETTE_ROWS)
            .map(|i| (i.title.to_string(), i.detail.clone()))
            .collect();
        self.palette.replace(Some(PaletteState {
            matches: ranked
                .iter()
                .take(Self::PALETTE_ROWS)
                .map(|i| i.action)
                .collect(),
            selected: 0,
        }));
        self.bar.set_palette_rows(&rows, 0);
    }

    /// Move the highlight by `delta`, wrapping at the ends.
    fn palette_move(self: &Rc<Self>, delta: isize) {
        let mut state = self.palette.borrow_mut();
        let Some(state) = state.as_mut() else {
            return;
        };
        let n = state.matches.len();
        if n == 0 {
            return;
        }
        state.selected = (state.selected as isize + delta).rem_euclid(n as isize) as usize;
        self.bar.set_palette_selected(state.selected);
    }

    /// Run the highlighted action. The bar closes first so actions
    /// that open the bar themselves (find, URL) start clean.
    fn run_palette_selected(self: &Rc<Self>) {
        let action = {
            let state = self.palette.borrow();
            state
                .as_ref()
                .and_then(|s| s.matches.get(s.selected).copied())
        };
        self.palette.replace(None);
        self.bar.close();
        self.focus_webview();
        if let Some(action) = action {
            self.run_action(action);
        }
    }

    /// Close this window if it is still an untouched launcher: showing
    /// the launcher page with no navigation history. Returns whether
    /// it closed.
    fn close_if_bare_launcher(self: &Rc<Self>) -> bool {
        let Some(webview) = self.live_webview() else {
            return false;
        };
        let on_launcher = webview.uri().is_some_and(|u| launcher::is_launcher(&u));
        if on_launcher && !webview.can_go_back() {
            self.window.close();
            return true;
        }
        false
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

    /// Open the current-URL prompt and seed it with a key that was typed
    /// while the page had focus, replacing the selected current URL.
    fn open_url_with_seed(self: &Rc<Self>, seed: &str) {
        self.open_url_bar();
        self.bar.entry.set_text(seed);
        self.bar.entry.set_position(-1);
    }

    /// Navigate this window, normalizing bare hosts the same way the
    /// CLI does (`example.com` -> `https://example.com`).
    fn navigate(self: &Rc<Self>, input: &str) {
        self.restore();
        if let Some(webview) = self.live_webview() {
            let url = crate::ipc_server::normalize_url(input.to_string());
            self.mark_nav_pending(&url);
            webview.load_uri(&url);
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
        // No committed search: let the repeat-search key through to the page.
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

    #[test]
    fn printable_key_text_excludes_modified_and_whitespace_keys() {
        assert_eq!(
            super::printable_key_text(gtk::gdk::Key::a, gtk::gdk::ModifierType::empty()),
            Some("a".to_string())
        );
        assert_eq!(
            super::printable_key_text(gtk::gdk::Key::a, gtk::gdk::ModifierType::CONTROL_MASK),
            None
        );
        assert_eq!(
            super::printable_key_text(gtk::gdk::Key::space, gtk::gdk::ModifierType::empty()),
            None
        );
    }

    #[test]
    fn quarter_width_uses_one_fourth_of_the_viewport() {
        assert_eq!(super::quarter_width(1920), 480);
        assert_eq!(super::quarter_width(1366), 341);
    }

    #[test]
    fn quarter_width_never_returns_zero() {
        assert_eq!(super::quarter_width(0), 1);
        assert_eq!(super::quarter_width(-1), 1);
    }
}
