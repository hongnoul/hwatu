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
use hwatu_ipc::{OpenMode, WebProcessTerminationInfo, WindowInfo};
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

/// WebKit cancels the in-flight main-frame request when another navigation
/// replaces it, including back/forward history traversal. That is normal
/// navigation control flow, not a page failure worth surfacing to the user.
fn load_was_cancelled(error: &glib::Error) -> bool {
    error.matches(webkit6::NetworkError::Cancelled)
        || error.matches(gtk::gio::IOErrorEnum::Cancelled)
}

/// Whether a navigation action is a conventional Ctrl+click on a link.
/// WebKit reports modifiers as GDK bits but leaves tab creation to the
/// embedder, so without this check Ctrl+click navigates the current view.
fn is_ctrl_click_link(
    navigation_type: webkit6::NavigationType,
    mouse_button: u32,
    modifiers: u32,
) -> bool {
    navigation_type == webkit6::NavigationType::LinkClicked
        && mouse_button == 1
        && modifiers & gtk::gdk::ModifierType::CONTROL_MASK.bits() != 0
}

/// Return the scheme for a URI that should leave the browser.
///
/// WebKit does not launch desktop handlers for custom schemes on behalf of
/// embedders. Keep browser-owned schemes in the WebView and hand protocols
/// such as `zoommtg:`, `mailto:`, and `tel:` to GIO instead. Callers must also
/// require a user gesture so a page cannot launch native applications merely
/// by loading.
fn external_uri_scheme(uri: &str) -> Option<String> {
    let (scheme, _) = uri.split_once(':')?;
    if scheme.is_empty()
        || !scheme.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphabetic()
                || (index > 0 && (byte.is_ascii_digit() || matches!(byte, b'+' | b'-' | b'.')))
        })
    {
        return None;
    }

    let scheme = scheme.to_ascii_lowercase();
    match scheme.as_str() {
        "http" | "https" | "about" | "data" | "blob" | "file" | "javascript" | "ws" | "wss"
        | "hwatu" => None,
        _ => Some(scheme),
    }
}

/// Media autoplay policy for every WebView. WebKit's own default is
/// ALLOW_WITHOUT_SOUND, which is why video sites autoplay muted: their
/// unmuted-play probe is rejected, so they fall back to a muted player.
/// hwatu defaults to full allow — sound is expected of a browser one
/// actually watches things in. Override with HWATU_AUTOPLAY=muted (the
/// WebKit default) or =deny (no autoplay at all), or persistently with
/// `"autoplay": "muted"|"deny"` in ~/.config/hwatu/config.json — env
/// vars silently vanish on daemon restarts, which repeatedly re-wedged
/// agents relying on deny to dodge the GStreamer deadlock below.
fn autoplay_policy() -> webkit6::AutoplayPolicy {
    // HWATU_BLOCK_AUTOPLAY=1 is the older escape hatch for a
    // WebKitGTK+GStreamer wedge (gst 1.28.5: pages with several
    // lazy-initialized autoplay videos deadlock the web process);
    // scripts still set it, so it stays as an alias for deny.
    if std::env::var_os("HWATU_BLOCK_AUTOPLAY").is_some_and(|v| v == "1") {
        return webkit6::AutoplayPolicy::Deny;
    }
    let env = std::env::var("HWATU_AUTOPLAY").ok();
    let choice = env.or_else(config_autoplay);
    match choice.as_deref() {
        Some("muted") | Some("without-sound") => webkit6::AutoplayPolicy::AllowWithoutSound,
        Some("deny") | Some("off") => webkit6::AutoplayPolicy::Deny,
        _ => webkit6::AutoplayPolicy::Allow,
    }
}

/// Read one key from ~/.config/hwatu/config.json (the same file adblock
/// persists its toggle in). Returns None when the file or key is
/// absent/invalid.
pub(crate) fn config_value(key: &str) -> Option<serde_json::Value> {
    let raw =
        std::fs::read_to_string(glib::user_config_dir().join("hwatu").join("config.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    Some(v.get(key)?.clone())
}

/// Read `"autoplay"` from config.json. Returns None when absent/invalid.
fn config_autoplay() -> Option<String> {
    Some(config_value("autoplay")?.as_str()?.to_string())
}

const DEFAULT_WINDOW_WIDTH: i32 = 1024;
const DEFAULT_WINDOW_HEIGHT: i32 = 768;

/// Request the preferred fraction of the current monitor's width for a newly
/// mapped window. Tiling WMs use this as the initial size hint when deciding
/// how to place a new toplevel, while floating WMs still get a useful
/// desktop-sized window instead of an arbitrary fixed width. The built-in
/// default is one half; users can override it with `"preferred_width"` in
/// `~/.config/hwatu/config.json`.
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

    preferred_width(monitor.geometry().width(), preferred_width_ratio())
}

const DEFAULT_PREFERRED_WIDTH: f64 = 1.0 / 2.0;

fn preferred_width_ratio() -> f64 {
    config_value("preferred_width")
        .and_then(|v| ratio_from_value(&v))
        .unwrap_or(DEFAULT_PREFERRED_WIDTH)
}

/// Accept only a finite fraction in (0, 1]; anything else (zero, negatives,
/// above one, NaN, inf, non-numbers) is rejected so a typo cannot produce
/// an invisible or absurd window.
fn ratio_from_value(value: &serde_json::Value) -> Option<f64> {
    value
        .as_f64()
        .filter(|ratio| ratio.is_finite() && *ratio > 0.0 && *ratio <= 1.0)
}

fn preferred_width(viewport_width: i32, ratio: f64) -> i32 {
    ((viewport_width as f64) * ratio).floor().max(1.0) as i32
}

/// Semantic app-id for a URL from `"app_ids"` in config.json (roadmap
/// H30): `{"app_ids": {"youtube.com": "hwatu.media", "github.com":
/// "hwatu.code"}}`. Keys match the host or any parent-domain suffix
/// (mail.google.com matches a google.com rule). Longest key wins.
fn site_app_id(url: &str) -> Option<String> {
    let rules = config_value("app_ids")?;
    let rules = rules.as_object()?;
    let host = crate::prompts::host_of(url).to_lowercase();
    if host.is_empty() {
        return None;
    }
    let mut best: Option<(&str, &str)> = None;
    for (key, value) in rules {
        let Some(id) = value.as_str() else { continue };
        let k = key.to_lowercase();
        if (host == k || host.ends_with(&format!(".{k}")))
            && best.is_none_or(|(bk, _)| k.len() > bk.len())
        {
            best = Some((key.as_str(), id));
        }
    }
    best.map(|(_, id)| id.to_string())
}

/// Known-graphical editors that wait for their window to close ("-w"
/// style). Anything else is assumed terminal-bound.
fn editor_is_graphical(editor: &str) -> bool {
    let bin = editor.split_whitespace().next().unwrap_or(editor);
    let bin = bin.rsplit('/').next().unwrap_or(bin);
    matches!(
        bin,
        "code" | "codium" | "subl" | "gedit" | "kate" | "zeditor" | "zed"
    )
}

/// Is `bin` on PATH?
fn which_exists(bin: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(bin).is_file()))
}

/// Interpret only the keys owned by a confirmation prompt. Everything else
/// must keep propagating to the page, otherwise a prompt appearing while the
/// user types into a form can silently eat characters.
fn confirm_answer(key: gtk::gdk::Key, state: gtk::gdk::ModifierType) -> Option<bool> {
    let command_modifiers = gtk::gdk::ModifierType::CONTROL_MASK
        | gtk::gdk::ModifierType::ALT_MASK
        | gtk::gdk::ModifierType::SUPER_MASK
        | gtk::gdk::ModifierType::HYPER_MASK
        | gtk::gdk::ModifierType::META_MASK;
    if state.intersects(command_modifiers) {
        return None;
    }
    match key {
        gtk::gdk::Key::y | gtk::gdk::Key::Y => Some(true),
        gtk::gdk::Key::n | gtk::gdk::Key::N | gtk::gdk::Key::Escape => Some(false),
        _ => None,
    }
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

fn termination_info(
    reason: webkit6::WebProcessTerminationReason,
    url: String,
) -> WebProcessTerminationInfo {
    let (reason, message) = match reason {
        webkit6::WebProcessTerminationReason::Crashed => ("crashed", "crashed"),
        webkit6::WebProcessTerminationReason::ExceededMemoryLimit => {
            ("oom", "was killed (out of memory)")
        }
        _ => ("terminated", "terminated unexpectedly"),
    };
    WebProcessTerminationInfo {
        reason: reason.to_string(),
        message: message.to_string(),
        url,
    }
}

fn recovery_url(live_url: Option<&str>, last_url: &str) -> Option<String> {
    live_url
        .filter(|url| !url.is_empty())
        .or((!last_url.is_empty()).then_some(last_url))
        .map(str::to_string)
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
    /// Last non-empty URL observed or requested for this window. WebKit can
    /// return an empty URI after web-process termination; keep enough state
    /// for diagnostics and recovery instead of reporting a blank page.
    last_url: RefCell<String>,
    /// Last non-empty title, for the same post-termination fallback as URL.
    last_title: RefCell<String>,
    /// Last web-process termination diagnostic. Kept in the window model so
    /// `hwatu list --json` exposes the reason even when hwatud was auto-
    /// spawned with stdout/stderr discarded.
    web_process_terminated: RefCell<Option<WebProcessTerminationInfo>>,
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
    /// Suppress a retry storm: Turnstile emits the same 600010 rejection
    /// repeatedly in one document.  Reset on the next top-level load.
    turnstile_handoff_offered: std::cell::Cell<bool>,
    /// Command-palette state while the bar is in Palette mode: the
    /// current ranked matches and which one is highlighted. Cleared on
    /// close so a reopened palette starts fresh.
    palette: RefCell<Option<PaletteState>>,
    /// URL-bar history completions (roadmap H9): ranked URL matches
    /// for the current entry text, and which (if any) is highlighted.
    /// `None` selection means Enter navigates the typed text.
    completions: RefCell<Option<CompletionState>>,
}

/// See [`BrowserWindow::palette`].
struct PaletteState {
    matches: Vec<keys::Action>,
    selected: usize,
}

/// See [`BrowserWindow::completions`].
struct CompletionState {
    urls: Vec<String>,
    selected: Option<usize>,
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
/// (`HWATU_WEBKIT_FEATURES`) win over these, so any baseline can still be
/// reversed for testing.
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
///
/// WebKitGTK 2.52 ships the Storage API and File System Access API but keeps
/// them disabled by default. Together these features provide the Origin
/// Private File System exposed by `navigator.storage.getDirectory()`. Modern
/// local-first applications use OPFS for private, origin-scoped persistence,
/// so a general-purpose browser should expose WebKit's native implementation.
const BASELINE_FEATURE_OVERRIDES: &[(&str, bool)] = &[
    ("PropagateDamagingInformation", false),
    ("StorageAPI", true),
    ("StorageAPIEstimate", true),
    ("FileSystem", true),
    ("FileSystemWritableStream", true),
];

/// `HWATU_MEDIA_STREAM=1|on|true` re-enables the MediaStream API
/// (getUserMedia/enumerateDevices), which is off by default; see the
/// wedge documented at the call site in `apply_view_settings`. The env
/// var wins for a single run; `"media_stream": true` in
/// ~/.config/hwatu/config.json persists the choice across daemon
/// restarts (needed for daily WebRTC call use, where an env var
/// silently vanishing on restart means a dead microphone mid-meeting).
fn media_stream_enabled() -> bool {
    if let Ok(v) = std::env::var("HWATU_MEDIA_STREAM") {
        return matches!(v.trim(), "1" | "on" | "true");
    }
    config_value("media_stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
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

/// Push a closing window onto the reopen stack (ctrl+shift+t), capped
/// at 10 like mainstream browsers keep reachable by key. Headless
/// windows belong to an agent's verification run, and blank/launcher
/// pages are not worth resurrecting; none are recorded.
fn remember_recently_closed(stack: &RefCell<Vec<crate::session::SessionEntry>>, info: WindowInfo) {
    if info.mode == OpenMode::Headless
        || info.url.is_empty()
        || info.url == "about:blank"
        || crate::launcher::is_launcher(&info.url)
    {
        return;
    }
    let mut closed = stack.borrow_mut();
    closed.push(crate::session::SessionEntry {
        url: info.url,
        title: info.title,
        app_id: info.app_id,
        mode: info.mode,
    });
    let overflow = closed.len().saturating_sub(10);
    if overflow > 0 {
        closed.drain(..overflow);
    }
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
pub fn build_webview(daemon: &Daemon) -> webkit6::WebView {
    build_webview_with_session(daemon.network_session.as_ref())
}

/// [`build_webview`] against an explicit network session (platform
/// item 6, profiles): profiled windows can't adopt the prewarmed
/// pool view (it rides the default session), so they build here.
pub fn build_webview_with_session(session: Option<&webkit6::NetworkSession>) -> webkit6::WebView {
    // website-policies is construct-only, hence the builder. Autoplay
    // defaults to full allow (with sound); see autoplay_policy().
    let policies = webkit6::WebsitePolicies::builder()
        .autoplay(autoplay_policy())
        .build();
    let mut builder = webkit6::WebView::builder().website_policies(&policies);
    if let Some(session) = session {
        builder = builder.network_session(session);
    }
    let view = builder.build();
    apply_view_settings(&view);
    crate::console::wire_view(&view);
    crate::clock::wire_view(&view);
    crate::smoothwheel::wire_view(&view);
    crate::focusshield::wire_view(&view);
    crate::blurshield::wire_view(&view);
    crate::mediashim::wire_view(&view);
    crate::opfs::wire_view(&view);
    crate::trusted_input::wire_view(&view);
    crate::reader::wire_view(&view);
    // Link hints (roadmap H10): yank mode hands the href to the GDK
    // clipboard, page JS never touches navigator.clipboard.
    crate::hints::wire_view(&view, |href| {
        if let Some(display) = gtk::gdk::Display::default() {
            display.clipboard().set_text(&href);
            println!("hwatud: yanked {href}");
        }
    });
    view
}

/// Shared engine settings, applied to prewarmed views and to popup
/// views built with `related_view` (which bypass `build_webview`).
fn apply_view_settings(view: &webkit6::WebView) {
    if let Some(settings) = webkit6::prelude::WebViewExt::settings(view) {
        settings.set_enable_developer_extras(true);
        // Render as intended: leave JS, media, canvas, webgl at defaults.
        settings.set_enable_page_cache(true); // bfcache

        // Escape hatch for GStreamer wedges: some pages' background
        // videos deadlock the web process main thread inside
        // MediaPlayerPrivateGStreamer::changePipelineState (observed
        // on scale.com mp4 buffering: the run loop never returns, so
        // every eval/screenshot hangs and the WatchDogQueue SIGABRTs
        // the process). HWATU_DISABLE_MEDIA=1 disables media playback
        // entirely so automation against such pages stays responsive.
        if std::env::var("HWATU_DISABLE_MEDIA").is_ok_and(|v| !v.is_empty() && v != "0") {
            settings.set_enable_media(false);
        }

        // MediaStream (getUserMedia/enumerateDevices) is off unless
        // explicitly requested. On WebKitGTK 2.52.5 + GStreamer 1.28
        // (pipewire provider), a page calling
        // navigator.mediaDevices.enumerateDevices() wedges the web
        // process main thread inside
        // GStreamerVideoCaptureDeviceManager::computeCaptureDevices —
        // a nested g_main_context_iteration loop that never completes,
        // pinning one core at 100% and starving JS/paint/IPC for the
        // life of the process (reproduced on example.com with a bare
        // enumerateDevices() call; observed in the wild via
        // doordash.com's fingerprinting SDK, which probes devices on
        // every page). Chromium-family browsers answer the same probe
        // in microseconds, which is why the same sites feel fine
        // there. With the API disabled, navigator.mediaDevices is
        // simply absent and such probes fall through their error
        // paths instantly. HWATU_MEDIA_STREAM=1 re-enables for actual
        // camera/mic use.
        if !media_stream_enabled() {
            settings.set_enable_media_stream(false);
        } else {
            // WebRTC (Meet/Discord/Zoom-web calls) defaults OFF in
            // WebKitGTK. enable-webrtc implies media-stream, so it
            // rides the same gate: flipping it while media-stream is
            // force-disabled would silently re-open the
            // enumerateDevices wedge documented above. Runtime codec
            // support needs the distro's gst-plugins-bad.
            settings.set_enable_webrtc(true);
        }

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
        Self::open_with_profile(daemon, url, app_id, mode, None)
    }

    /// [`Self::open`] with cookie/site-data isolation (platform item
    /// 6): `profile` names an isolated NetworkSession. Profiled
    /// windows skip the prewarmed pool (it rides the default
    /// session) and build a fresh view against the profile's session;
    /// isolation is worth the one-time engine setup cost.
    pub fn open_with_profile(
        daemon: &Rc<Daemon>,
        url: Option<String>,
        app_id: Option<String>,
        mode: OpenMode,
        profile: Option<String>,
    ) -> WindowInfo {
        // On Wayland the compositor decides who gets focus, and most
        // tilers focus new windows. A background window therefore gets
        // a predictable default app_id so one WM rule can opt it out
        // (niri: `match app-id="hwatu-background"` + `open-focused
        // false`; Hyprland: `windowrule = noinitialfocus, class:...`).
        //
        // Semantic app-ids (roadmap H30): with no explicit app_id, a
        // profiled window gets `hwatu.<profile>` and a URL matching an
        // `"app_ids": {"<host-suffix>": "<id>"}` config rule gets that
        // id — so niri window rules do auto-placement, floating, and
        // workspace pinning without hwatu growing its own rule engine.
        let app_id = app_id
            .or_else(|| {
                profile
                    .as_deref()
                    .filter(|p| !p.is_empty())
                    .map(|p| format!("hwatu.{p}"))
            })
            .or_else(|| url.as_deref().and_then(site_app_id))
            .or_else(|| (mode == OpenMode::Background).then(|| "hwatu-background".to_string()));
        let webview = match profile.as_deref().filter(|p| !p.is_empty()) {
            Some(name) => {
                let session = daemon.profile_session(name);
                let view = build_webview_with_session(Some(&session));
                daemon.adblock.apply_to(&view);
                view
            }
            None => daemon.take_webview(),
        };
        let this = Self::build(daemon, webview.clone(), app_id.clone(), mode);
        // No URL and no configured home page: show the launcher (the
        // keybind cheat sheet) with the URL bar already open, so a
        // bare `hwatu` is "type where you want to go".
        let target = match url.or_else(home_page) {
            Some(url) => {
                this.mark_nav_pending(&url);
                this.remember_url(&url);
                webview.load_uri(&url);
                url
            }
            None => {
                let uri = launcher::deal_uri(daemon.take_deal());
                this.mark_nav_pending(&uri);
                this.remember_url(&uri);
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
            web_process_terminated: None,
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
        crate::opfs::wire_view(&webview);
        crate::trusted_input::wire_view(&webview);
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
            prompts: Prompts::new(daemon.site_store.clone()),
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
            last_url: RefCell::new(String::new()),
            last_title: RefCell::new(String::new()),
            web_process_terminated: RefCell::new(None),
            load_committed: std::cell::Cell::new(true),
            snapshot_baseline: RefCell::new(None),
            console: crate::console::Buffer::default(),
            net: crate::net::Buffer::default(),
            turnstile_handoff_offered: std::cell::Cell::new(false),
            palette: RefCell::new(None),
            completions: RefCell::new(None),
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
            let weak = Rc::downgrade(&this);
            this.console.set_hook(move |entry| {
                daemon.events.emit(
                    "console",
                    Some(win_id),
                    serde_json::to_value(entry).unwrap_or_default(),
                );
                if crate::console::is_turnstile_compat_error(entry) {
                    let Some(this) = weak.upgrade() else { return };
                    let Some(uri) = entry
                        .page
                        .as_deref()
                        .filter(|uri| uri.starts_with("https://") || uri.starts_with("http://"))
                    else {
                        return;
                    };
                    // Invalid/non-web page provenance must not consume the
                    // one-shot offer: a later valid main-page rejection in
                    // this document still deserves a recovery prompt.
                    if this.turnstile_handoff_offered.replace(true) {
                        return;
                    }
                    this.push_prompt(Prompt::ExternalBrowser {
                        uri: uri.to_string(),
                    });
                }
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
            let weak = Rc::downgrade(&this);
            ctrl.connect_key_pressed(move |_, key, _, state| {
                weak.upgrade().map_or(glib::Propagation::Proceed, |this| {
                    this.on_window_key(key, state)
                })
            });
            window.add_controller(ctrl);
        }

        // Capture-phase keys: things that must win over the page, which
        // keeps focus and would otherwise swallow them before bubble.
        // Modified chords (ctrl/alt) dispatch here via the keymap;
        // y/n/Esc while the bar is in confirm mode are fixed keys.
        {
            let ctrl = gtk::EventControllerKey::new();
            ctrl.set_propagation_phase(gtk::PropagationPhase::Capture);
            let weak = Rc::downgrade(&this);
            ctrl.connect_key_pressed(move |_, key, _, state| {
                let Some(this) = weak.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                // While the bar's entry owns focus (find/URL/palette
                // typing), global chords stay out of the way: Ctrl+o in
                // a URL prompt must not navigate history under the bar.
                let entry_open = matches!(
                    this.bar.mode(),
                    BarMode::Find { .. } | BarMode::Url | BarMode::Palette
                );
                if !entry_open {
                    if let Some(action) =
                        this.daemon.keymap.lookup(keys::Phase::Capture, key, state)
                    {
                        return this.run_action(action);
                    }
                }
                let BarMode::Confirm { tag } = this.bar.mode() else {
                    return glib::Propagation::Proceed;
                };
                let Some(answer) = confirm_answer(key, state) else {
                    return glib::Propagation::Proceed;
                };
                this.answer_confirm(&tag, answer);
                glib::Propagation::Stop
            });
            window.add_controller(ctrl);
        }

        // Focus-driven lifecycle: unfocused windows are scheduled for
        // discard, focused ones are restored immediately.
        {
            let weak = Rc::downgrade(&this);
            window.connect_is_active_notify(move |win| {
                let Some(this) = weak.upgrade() else { return };
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
            let weak = Rc::downgrade(&this);
            window.connect_close_request(move |_| {
                let Some(this) = weak.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                this.cancel_discard_timer();
                // Remember the page for ctrl+shift+t before the window
                // leaves the registry.
                remember_recently_closed(&daemon.recently_closed, this.info());
                // The GTK toplevel dying does not kill the web process:
                // WebKit caches it for reuse, and a cached process keeps
                // running its page — an autoplaying video's audio audibly
                // outlives the window (and the agent session that opened
                // it). Kill it here, on the same shared-process guard as
                // the discard path.
                this.detach_webview_and_terminate_unless_shared();
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
        let this = self.clone();
        webview.connect_title_notify(move |wv| {
            let title = wv.title().unwrap_or_default();
            if !title.is_empty() {
                this.last_title.replace(title.to_string());
                // History completion shows titles (roadmap H9).
                if this.mode.get() != OpenMode::Headless {
                    if let Some(url) = wv.uri() {
                        this.daemon.history.record_title(&url, &title);
                    }
                }
            }
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
            let this = self.clone();
            webview.connect_uri_notify(move |wv| {
                this.remember_webview_url(wv);
                daemon.schedule_session_save();
            });
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
                    this.web_process_terminated.replace(None);
                    this.turnstile_handoff_offered.set(false);
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
                    // Per-site zoom (roadmap H5): apply the remembered
                    // level for the new document's host. Explicitly
                    // reset to 1.0 otherwise, or a zoom picked up on
                    // one site follows the window to the next.
                    let host = prompts::host_of(&wv.uri().unwrap_or_default());
                    let level = this.daemon.site_store.zoom(&host).unwrap_or(1.0);
                    if (wv.zoom_level() - level).abs() > 0.001 {
                        wv.set_zoom_level(level);
                    }
                    // Forced dark mode (roadmap H15): re-apply the
                    // per-site/global preference on the new document.
                    if crate::darkmode::should_darken(&this.daemon.site_store, &host) {
                        wv.evaluate_javascript(
                            &crate::darkmode::apply_js(true),
                            None,
                            None,
                            gtk::gio::Cancellable::NONE,
                            |_| {},
                        );
                    }
                    // Global history (roadmap H9): record user
                    // navigations. Headless windows belong to agent
                    // verification runs; internal pages and blanks
                    // are not completions anyone wants.
                    if this.mode.get() != OpenMode::Headless {
                        if let Some(url) = wv.uri() {
                            if !url.is_empty()
                                && url != "about:blank"
                                && !launcher::is_launcher(&url)
                            {
                                this.daemon.history.record_visit(&url);
                            }
                        }
                    }
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
        // File uploads (roadmap H1): <input type=file> emits
        // run-file-chooser; unhandled, WebKit's default silently
        // cancels and every upload flow is dead. Open a GTK file
        // dialog honoring the input's MIME filter and multiple flag.
        {
            let this = self.clone();
            webview.connect_run_file_chooser(move |_, request| {
                let dialog = gtk::FileDialog::builder()
                    .title("Upload file")
                    .modal(true)
                    .build();
                if let Some(filter) = request.mime_types_filter() {
                    filter.set_name(Some("Accepted files"));
                    let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
                    let all = gtk::FileFilter::new();
                    all.set_name(Some("All files"));
                    all.add_pattern("*");
                    filters.append(&filter);
                    filters.append(&all);
                    dialog.set_filters(Some(&filters));
                    dialog.set_default_filter(Some(&filter));
                }
                let request = request.clone();
                let paths_of = |files: gtk::gio::ListModel| -> Vec<String> {
                    (0..files.n_items())
                        .filter_map(|i| files.item(i))
                        .filter_map(|f| f.downcast::<gtk::gio::File>().ok())
                        .filter_map(|f| f.path())
                        .map(|p| p.to_string_lossy().into_owned())
                        .collect()
                };
                if request.selects_multiple() {
                    dialog.open_multiple(
                        Some(&this.window),
                        gtk::gio::Cancellable::NONE,
                        move |result| match result.map(paths_of) {
                            Ok(paths) if !paths.is_empty() => {
                                let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
                                request.select_files(&refs);
                            }
                            _ => request.cancel(),
                        },
                    );
                } else {
                    dialog.open(
                        Some(&this.window),
                        gtk::gio::Cancellable::NONE,
                        move |result| match result.ok().and_then(|f| f.path()) {
                            Some(path) => {
                                request.select_files(&[&path.to_string_lossy()]);
                            }
                            None => request.cancel(),
                        },
                    );
                }
                true // handled
            });
        }
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
                // Back/forward (and any navigation that supersedes an
                // in-flight load) cancels the old main-frame request. The
                // replacement load is already underway, so treating this as
                // a failure leaves a false overlay on top of the new page.
                if load_was_cancelled(error) {
                    return true;
                }
                // A navigation converted into a download (attachment
                // disposition or unrenderable MIME) aborts the frame
                // load with FrameLoadInterruptedByPolicyChange. That
                // is not a failure - the bytes are arriving via the
                // download machinery - so a "Page failed to load"
                // overlay would be a lie. Flash the bar instead; the
                // download wiring reports the saved path when done.
                if error.matches(webkit6::PolicyError::FrameLoadInterruptedByPolicyChange) {
                    this.flash_bar("downloading…", 4);
                    return true;
                }
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
                let url = wv
                    .uri()
                    .map(|u| u.to_string())
                    .filter(|u| !u.is_empty())
                    .unwrap_or_else(|| this.last_url.borrow().clone());
                let info = termination_info(reason, url.clone());
                this.web_process_terminated.replace(Some(info.clone()));
                this.daemon.events.emit(
                    "web_process",
                    Some(this.id),
                    serde_json::json!({
                        "state": "terminated",
                        "reason": info.reason,
                        "message": info.message,
                        "url": info.url,
                    }),
                );
                eprintln!(
                    "hwatud: web process for window {} {} at {}",
                    this.id,
                    info.message,
                    if url.is_empty() { "(unknown URL)" } else { &url }
                );
                this.show_recovery_overlay(
                    "Page crashed",
                    &format!("The web process {}. Press Ctrl+r or F5 to reload, or Ctrl+l to open a URL.", info.message),
                    RecoveryOverlay::Failure,
                );
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
        // window.print() (roadmap H7): route the page-initiated print
        // through the same dialog as ctrl+p.
        {
            let this = self.clone();
            webview.connect_print(move |_, operation| {
                this.run_print_operation(operation);
                true // handled; WebKit must not run its own dialog too
            });
        }
        // Web notifications (roadmap H4): forward to the desktop over
        // D-Bus; a click presents this window (an explicit request).
        {
            let this = self.clone();
            crate::notify::wire_view(&webview, move || this.window.present());
        }
        // Ctrl+click follows the same client-tab path as Ctrl+t instead of
        // replacing the current page. Non-displayable responses
        // (Content-Disposition: attachment, MIME types WebKit can't render)
        // become downloads instead of dead ends. Main-document responses
        // also pass through the per-site UA switcher (mobile UI for
        // reels-style sites), which may restart the load under the right
        // user-agent.
        let this = self.clone();
        webview.connect_decide_policy(move |wv, decision, decision_type| {
            if matches!(
                decision_type,
                webkit6::PolicyDecisionType::NavigationAction
                    | webkit6::PolicyDecisionType::NewWindowAction
            ) {
                let Some(navigation) =
                    decision.dynamic_cast_ref::<webkit6::NavigationPolicyDecision>()
                else {
                    return false;
                };
                let Some(mut action) = navigation.navigation_action() else {
                    return false;
                };
                let Some(uri) = action.request().and_then(|request| request.uri()) else {
                    return false;
                };

                // Native-app links need explicit embedder handling. Only
                // honor them during a user gesture, and only when the desktop
                // has a registered handler for the scheme. This covers Zoom's
                // `zoommtg:` join button without allowing pages to launch
                // arbitrary programs on load.
                if action.is_user_gesture()
                    && external_uri_scheme(&uri).is_some_and(|scheme| {
                        gtk::gio::AppInfo::default_for_uri_scheme(&scheme).is_some()
                    })
                {
                    decision.ignore();
                    if let Err(error) = gtk::gio::AppInfo::launch_default_for_uri(
                        &uri,
                        None::<&gtk::gio::AppLaunchContext>,
                    ) {
                        eprintln!("hwatud: could not open external URI: {error}");
                        this.flash_bar("could not open external application", 4);
                    }
                    return true;
                }

                if decision_type == webkit6::PolicyDecisionType::NewWindowAction {
                    return false;
                }

                if !is_ctrl_click_link(
                    action.navigation_type(),
                    action.mouse_button(),
                    action.modifiers(),
                ) {
                    return false;
                }
                decision.ignore();
                Self::open(&this.daemon, Some(uri.to_string()), None, OpenMode::Normal);
                return true;
            }
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
        // give memory back. Write failures degrade to a URL reload. In
        // ephemeral-profile mode no normal profile/cache state may be
        // written, so discards deliberately keep only the URL/title.
        let session_file = if self.daemon.security.ephemeral_profile {
            None
        } else {
            webview
                .session_state()
                .and_then(|state| state.serialize())
                .and_then(|bytes| {
                    let path = discard_dir()?.join(format!("window-{}.session", self.id));
                    std::fs::write(&path, bytes).ok()?;
                    Some(path)
                })
        };
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

    /// Current show mode (normal/background/headless).
    pub fn mode(&self) -> OpenMode {
        self.mode.get()
    }

    /// Raise and focus, promoting background/headless windows to
    /// normal: an explicit focus request means "show me this window".
    /// The prior mode is remembered so `unfocus` (or the auto-demote
    /// watchdog) can put the window back out of the user's way, and a
    /// watchdog is armed: a promoted window that no longer shows any
    /// need for a human (not focused, no bar prompt, no CAPTCHA)
    /// demotes itself instead of squatting in the WM forever.
    pub fn present(self: &Rc<Self>) {
        // Do not depend on the compositor granting activation (and emitting
        // is-active-notify) to preserve the promoted page. Cancel a pending
        // discard and restore before mapping the window.
        self.cancel_discard_timer();
        self.restore();
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
            Some(wv) => {
                let url = wv.uri().map(|u| u.to_string()).unwrap_or_default();
                WindowInfo {
                    id: self.id,
                    url: if url.is_empty() {
                        self.last_url.borrow().clone()
                    } else {
                        url
                    },
                    title: wv
                        .title()
                        .map(|t| t.to_string())
                        .filter(|title| !title.is_empty())
                        .unwrap_or_else(|| self.last_title.borrow().clone()),
                    focused: self.window.is_active(),
                    suspended: false,
                    app_id: self.app_id.clone(),
                    mode: self.mode.get(),
                    web_process_terminated: self
                        .web_process_terminated
                        .borrow()
                        .clone()
                        .map(Box::new),
                }
            }
            None => {
                let saved = self.saved.borrow();
                let (url, title) = saved
                    .as_ref()
                    .map(|s| (s.url.clone(), s.title.clone()))
                    .unwrap_or_default();
                WindowInfo {
                    id: self.id,
                    url: if url.is_empty() {
                        self.last_url.borrow().clone()
                    } else {
                        url
                    },
                    title: if title.is_empty() {
                        self.last_title.borrow().clone()
                    } else {
                        title
                    },
                    focused: false,
                    suspended: true,
                    app_id: self.app_id.clone(),
                    mode: self.mode.get(),
                    web_process_terminated: self
                        .web_process_terminated
                        .borrow()
                        .clone()
                        .map(Box::new),
                }
            }
        }
    }

    pub fn close(&self) {
        self.window.close();
    }

    /// Detach and drop this window's WebView, killing its web process unless
    /// a related window (popup ↔ opener) still runs in it.
    ///
    /// `terminate_web_process()` alone is not enough: every WebView signal
    /// owns callbacks that reference the BrowserWindow. Leaving the WebView
    /// in `self.webview` after close forms a Rust/GObject reference cycle,
    /// which lets WebKit retain (or restart) the supposedly closed page
    /// process. Detaching the widget and taking the field breaks that cycle.
    fn detach_webview_and_terminate_unless_shared(&self) {
        let webview = self.webview.borrow_mut().take();
        self.overlay.set_child(None::<&gtk::Widget>);
        let shared = self
            .process_group
            .borrow_mut()
            .take()
            .is_some_and(|group| Rc::strong_count(&group) > 1);
        if shared {
            return;
        }
        if let Some(webview) = webview {
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

    fn remember_webview_url(&self, webview: &webkit6::WebView) {
        if let Some(uri) = webview.uri().filter(|uri| !uri.is_empty()) {
            self.remember_url(uri.as_str());
        }
    }

    fn remember_url(&self, url: &str) {
        if !url.is_empty() {
            self.last_url.replace(url.to_string());
        }
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
            Action::Mute => self.toggle_mute(),
            Action::ReopenClosed => self.reopen_closed(),
            Action::Print => self.print_page(),
            Action::HintFollow => self.start_hints("follow"),
            Action::HintNewWindow => self.start_hints("newwin"),
            Action::HintYank => self.start_hints("yank"),
            Action::FillPassword => self.fill_password(),
            Action::DarkMode => self.toggle_dark_mode(),
            Action::OpenMpv => self.open_mpv(),
            Action::EditInEditor => self.edit_in_editor(),
            Action::Reader => self.toggle_reader(),
            Action::Share => self.open_share_menu(),
            Action::CommandPalette => self.open_palette(),
        }
        glib::Propagation::Stop
    }

    /// Toggle the page's own video mute — the same state a reels/player
    /// UI shows — not the WebView-level audio kill switch, which page
    /// mute buttons don't reflect. Picks the most prominent visible
    /// video (playing first, then largest on screen), flips `.muted`,
    /// and flashes the result. Flashes "no video" when the page has
    /// none; a discarded window has no page and this is a no-op.
    ///
    /// The choice persists across videos: feeds like Instagram reels
    /// mount each clip as a fresh muted `<video>`, so the first toggle
    /// installs a capture-phase `play` listener that re-applies the
    /// preferred state to every video that starts. Site-side mute
    /// clicks are adopted as the new preference (a click handler
    /// re-reads the state) so the key and the page UI never fight.
    fn toggle_mute(self: &Rc<Self>) {
        let Some(webview) = self.live_webview() else {
            return;
        };
        let js = r#"(() => {
            const S = window.__hwatuMute ??= { muted: null, installed: false };
            const inView = (v) => {
                const r = v.getBoundingClientRect();
                return r.width > 0 && r.height > 0 && r.bottom > 0 &&
                    r.right > 0 && r.top < innerHeight && r.left < innerWidth;
            };
            const visArea = (v) => {
                const r = v.getBoundingClientRect();
                return Math.max(0, Math.min(r.right, innerWidth) - Math.max(r.left, 0)) *
                    Math.max(0, Math.min(r.bottom, innerHeight) - Math.max(r.top, 0));
            };
            const pick = () => {
                const vids = [...document.querySelectorAll('video')].filter(inView);
                vids.sort((a, b) => (a.paused - b.paused) || (visArea(b) - visArea(a)));
                return vids[0];
            };
            const apply = (v) => {
                v.muted = S.muted;
                if (!S.muted && v.volume === 0) v.volume = 1;
            };
            const v = pick();
            if (!v) return 'no video';
            S.muted = !v.muted;
            apply(v);
            if (!S.installed) {
                S.installed = true;
                document.addEventListener('play', (e) => {
                    if (e.target instanceof HTMLVideoElement && S.muted !== null &&
                        e.target.muted !== S.muted) {
                        apply(e.target);
                    }
                }, true);
                document.addEventListener('click', () => {
                    setTimeout(() => {
                        const v = pick();
                        if (v) S.muted = v.muted;
                    }, 100);
                }, true);
            }
            return S.muted ? 'muted' : 'unmuted';
        })()"#;
        let this = self.clone();
        webview.evaluate_javascript(js, None, None, gtk::gio::Cancellable::NONE, move |result| {
            let msg = result
                .ok()
                .map(|v| v.to_str().to_string())
                .unwrap_or_else(|| "mute toggle failed".into());
            this.flash_bar(&msg, 2);
        });
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
        if webview.uri().is_none_or(|uri| uri.is_empty()) {
            let last_url = self.last_url.borrow().clone();
            if let Some(url) = recovery_url(None, &last_url) {
                self.mark_nav_pending(&url);
                webview.load_uri(&url);
            }
            return;
        }
        if bypass_cache {
            webview.reload_bypass_cache();
        } else {
            webview.reload();
        }
    }

    /// Multiply the page zoom by `factor`, clamped to a sane range.
    /// Zoom is per-window state on the WebView, like other browsers,
    /// and the resulting level persists per-site (roadmap H5).
    fn zoom_by(self: &Rc<Self>, factor: f64) {
        let Some(webview) = self.live_webview() else {
            return;
        };
        let level = (webview.zoom_level() * factor).clamp(0.25, 5.0);
        webview.set_zoom_level(level);
        self.remember_zoom(&webview, level);
        self.flash_bar(&format!("zoom {:.0}%", level * 100.0), 1);
    }

    /// Back to 100% (ctrl+0).
    fn zoom_reset(self: &Rc<Self>) {
        let Some(webview) = self.live_webview() else {
            return;
        };
        webview.set_zoom_level(1.0);
        self.remember_zoom(&webview, 1.0);
        self.flash_bar("zoom 100%", 1);
    }

    /// Persist the zoom preference for the current page's host.
    fn remember_zoom(&self, webview: &webkit6::WebView, level: f64) {
        let host = prompts::host_of(&webview.uri().unwrap_or_default());
        if !host.is_empty() && !host.starts_with("hwatu") {
            self.daemon.site_store.set_zoom(&host, level);
        }
    }

    /// Print the page (roadmap H7): system print dialog on the
    /// current window; print-to-PDF comes free with GTK's dialog.
    /// Sites that call window.print() land here too via the
    /// WebView's `print` signal.
    fn print_page(self: &Rc<Self>) {
        let Some(webview) = self.live_webview() else {
            return;
        };
        let operation = webkit6::PrintOperation::new(&webview);
        self.run_print_operation(&operation);
    }

    /// Run one print dialog, reporting failures in the bar.
    fn run_print_operation(self: &Rc<Self>, operation: &webkit6::PrintOperation) {
        let this = self.clone();
        operation.connect_failed(move |_, error| {
            eprintln!("hwatud: print failed: {error}");
            this.flash_bar("print failed", 4);
        });
        operation.run_dialog(Some(&self.window));
    }

    /// Enter link-hint mode (roadmap H10). `mode`: follow | newwin |
    /// yank. The page-side machinery reports "no hints" when the page
    /// has no visible interactables; surface that instead of silence.
    fn start_hints(self: &Rc<Self>, mode: &str) {
        let Some(webview) = self.live_webview() else {
            return;
        };
        let this = self.clone();
        webview.evaluate_javascript(
            &crate::hints::start_js(mode),
            None,
            None,
            gtk::gio::Cancellable::NONE,
            move |result| {
                if let Ok(value) = result {
                    if value.is_string() && value.to_str() == "no hints" {
                        this.flash_bar("no hints on this page", 1);
                    }
                }
            },
        );
    }

    /// Toggle forced dark mode for the current site (roadmap H15).
    /// The new state persists per host on the site store and re-applies
    /// on every future navigation to it.
    fn toggle_dark_mode(self: &Rc<Self>) {
        let Some(webview) = self.live_webview() else {
            return;
        };
        let host = prompts::host_of(&webview.uri().unwrap_or_default());
        if host.is_empty() || host.starts_with("hwatu") {
            self.flash_bar("no site to darken", 2);
            return;
        }
        let on = !crate::darkmode::should_darken(&self.daemon.site_store, &host);
        self.daemon.site_store.set_dark_mode(&host, on);
        let this = self.clone();
        webview.evaluate_javascript(
            &crate::darkmode::apply_js(on),
            None,
            None,
            gtk::gio::Cancellable::NONE,
            move |_| {
                this.flash_bar(
                    if on {
                        "dark mode on (this site)"
                    } else {
                        "dark mode off (this site)"
                    },
                    2,
                );
            },
        );
    }

    /// Hand the current page URL to mpv (roadmap H17): the loved
    /// mitigation for engine video gaps (DRM, codec walls). Detached
    /// spawn; mpv owns its own lifecycle.
    fn open_mpv(self: &Rc<Self>) {
        let Some(webview) = self.live_webview() else {
            return;
        };
        let Some(url) = webview.uri().filter(|u| u.starts_with("http")) else {
            self.flash_bar("no page URL for mpv", 2);
            return;
        };
        match std::process::Command::new("mpv")
            .arg(url.as_str())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(_) => self.flash_bar("handed to mpv", 2),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.flash_bar("mpv not installed", 4);
            }
            Err(e) => {
                eprintln!("hwatud: mpv spawn failed: {e}");
                self.flash_bar("mpv failed to start", 4);
            }
        }
    }

    /// Edit the focused text field in $EDITOR (roadmap H18): dump the
    /// field's value to a temp file, open $EDITOR (terminal editors
    /// get a terminal via $TERMINAL/foot/alacritty/kitty), and paste
    /// the saved contents back with framework-safe value setting.
    /// The watcher polls the file's mtime; closing the editor without
    /// saving changes nothing.
    fn edit_in_editor(self: &Rc<Self>) {
        let Some(webview) = self.live_webview() else {
            return;
        };
        // 1. Read the focused editable's value (fail open when the
        // focus isn't editable).
        let this = self.clone();
        webview.evaluate_javascript(
            r#"(() => {
  const el = document.activeElement;
  if (!el) return null;
  if (el.matches('textarea, input[type=text], input[type=search], input:not([type])'))
    return el.value;
  if (el.isContentEditable) return el.innerText;
  return null;
})()"#,
            None,
            None,
            gtk::gio::Cancellable::NONE,
            move |result| {
                let Ok(value) = result else { return };
                if value.is_null() {
                    this.flash_bar("focus a text field first", 2);
                    return;
                }
                let text = value.to_str().to_string();
                this.spawn_editor(text);
            },
        );
    }

    /// Phase 2 of [`Self::edit_in_editor`]: editor round-trip.
    fn spawn_editor(self: &Rc<Self>, initial: String) {
        let path =
            std::env::temp_dir().join(format!("hwatu-edit-{}-{}.txt", std::process::id(), self.id));
        if std::fs::write(&path, &initial).is_err() {
            self.flash_bar("cannot write temp file", 4);
            return;
        }
        let editor = std::env::var("VISUAL")
            .or_else(|_| std::env::var("EDITOR"))
            .unwrap_or_else(|_| "vi".to_string());
        // Terminal editors need a terminal. GUI editors ($VISUAL set
        // to e.g. "code -w") mostly cope with being spawned directly.
        let mut cmd = if editor_is_graphical(&editor) {
            let mut parts = editor.split_whitespace();
            let mut cmd = std::process::Command::new(parts.next().unwrap_or("vi"));
            cmd.args(parts);
            cmd.arg(&path);
            cmd
        } else {
            let terminal = std::env::var("TERMINAL").ok().or_else(|| {
                ["foot", "alacritty", "kitty", "wezterm"]
                    .iter()
                    .find(|t| which_exists(t))
                    .map(|t| t.to_string())
            });
            let Some(terminal) = terminal else {
                self.flash_bar("no terminal for $EDITOR (set $TERMINAL)", 4);
                return;
            };
            let mut cmd = std::process::Command::new(terminal);
            cmd.arg("-e");
            let mut parts = editor.split_whitespace();
            if let Some(bin) = parts.next() {
                cmd.arg(bin);
                cmd.args(parts);
            }
            cmd.arg(&path);
            cmd
        };
        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(e) => {
                eprintln!("hwatud: editor spawn failed: {e}");
                self.flash_bar("editor failed to start", 4);
                let _ = std::fs::remove_file(&path);
                return;
            }
        };
        self.flash_bar("editing… save + close to paste back", 3);
        // Poll for editor exit on the GTK loop (no blocking wait).
        let this = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(300), move || {
            match child.try_wait() {
                Ok(None) => glib::ControlFlow::Continue,
                Ok(Some(status)) => {
                    let text = std::fs::read_to_string(&path).unwrap_or_default();
                    let _ = std::fs::remove_file(&path);
                    if status.success() {
                        this.paste_into_focused(&text);
                    } else {
                        this.flash_bar("editor exited nonzero; not pasted", 3);
                    }
                    glib::ControlFlow::Break
                }
                Err(_) => {
                    let _ = std::fs::remove_file(&path);
                    glib::ControlFlow::Break
                }
            }
        });
    }

    /// Write `text` into the currently focused editable, framework-safe.
    fn paste_into_focused(self: &Rc<Self>, text: &str) {
        let Some(webview) = self.live_webview() else {
            return;
        };
        let payload = serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into());
        let js = format!(
            r#"(() => {{
  const TEXT = {payload};
  const el = document.activeElement;
  if (!el) return 'no focus';
  if (el.isContentEditable) {{ el.innerText = TEXT; return 'pasted'; }}
  if (!el.matches('textarea, input')) return 'not editable';
  const proto = Object.getPrototypeOf(el);
  const desc = Object.getOwnPropertyDescriptor(proto, 'value')
    || Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value');
  if (desc && desc.set) desc.set.call(el, TEXT); else el.value = TEXT;
  el.dispatchEvent(new Event('input', {{ bubbles: true }}));
  el.dispatchEvent(new Event('change', {{ bubbles: true }}));
  return 'pasted';
}})()"#
        );
        let this = self.clone();
        webview.evaluate_javascript(
            &js,
            None,
            None,
            gtk::gio::Cancellable::NONE,
            move |result| {
                let msg = match result {
                    Ok(v) if v.is_string() => v.to_str().to_string(),
                    _ => "paste failed".to_string(),
                };
                this.flash_bar(&msg, 2);
            },
        );
    }

    /// Toggle reader mode (roadmap H34): article extraction into a
    /// clean-typography overlay; Esc or re-toggle exits, original DOM
    /// untouched. The page-side machinery reports what happened.
    fn toggle_reader(self: &Rc<Self>) {
        let Some(webview) = self.live_webview() else {
            return;
        };
        let this = self.clone();
        webview.evaluate_javascript(
            crate::reader::toggle_js(),
            None,
            None,
            gtk::gio::Cancellable::NONE,
            move |result| {
                if let Ok(value) = result {
                    if value.is_string() {
                        this.flash_bar(&value.to_str(), 2);
                    }
                }
            },
        );
    }

    /// Share the page URL (roadmap H36): one share.conf target runs
    /// directly; several open the palette-style list; none explains
    /// how to configure.
    fn open_share_menu(self: &Rc<Self>) {
        let Some(webview) = self.live_webview() else {
            return;
        };
        let Some(url) = webview.uri().filter(|u| u.starts_with("http")) else {
            self.flash_bar("no page URL to share", 2);
            return;
        };
        let targets = crate::share::targets();
        match targets.len() {
            0 => self.flash_bar("no share targets (write ~/.config/hwatu/share.conf)", 4),
            _ => {
                // Reuse the bar's row surface as a picker: rows are
                // (name, command head); Up/Down+Enter via the palette
                // machinery would need a dedicated mode, so run the
                // first target directly for 1 and list names for >1
                // via sequential flash. Simplicity beats a modal here;
                // heavy users bind `share` per target in share.conf
                // order (share1..shareN planned if demand appears).
                let target = &targets[0];
                match crate::share::run(target, &url) {
                    Ok(()) => self.flash_bar(&format!("shared via {}", target.name), 2),
                    Err(e) => self.flash_bar(&e, 4),
                }
            }
        }
    }

    /// Fill login credentials from the system password manager
    /// (roadmap H11). The backend query runs on a worker thread —
    /// gpg pinentry can take seconds — and the fill JS runs back on
    /// the GTK thread. Secrets never hit logs or the bar.
    fn fill_password(self: &Rc<Self>) {
        let Some(webview) = self.live_webview() else {
            return;
        };
        let host = prompts::host_of(&webview.uri().unwrap_or_default());
        if host.is_empty() || host.starts_with("hwatu") {
            self.flash_bar("no site to fill", 2);
            return;
        }
        self.flash_bar("looking up credentials…", 2);
        let (tx, rx) = std::sync::mpsc::channel();
        {
            let host = host.clone();
            std::thread::spawn(move || {
                let _ = tx.send(crate::passfill::lookup(&host));
            });
        }
        let this = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            match rx.try_recv() {
                Ok(Ok(credential)) => {
                    let Some(webview) = this.live_webview() else {
                        return glib::ControlFlow::Break;
                    };
                    let this2 = this.clone();
                    webview.evaluate_javascript(
                        &crate::passfill::fill_js(&credential),
                        None,
                        None,
                        gtk::gio::Cancellable::NONE,
                        move |result| {
                            let msg = match result {
                                Ok(v) if v.is_string() => v.to_str().to_string(),
                                _ => "fill failed".to_string(),
                            };
                            this2.flash_bar(&msg, 2);
                        },
                    );
                    glib::ControlFlow::Break
                }
                Ok(Err(error)) => {
                    this.flash_bar(&error.to_string(), 4);
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    this.flash_bar("password lookup crashed", 4);
                    glib::ControlFlow::Break
                }
            }
        });
    }

    /// Reopen the most recently closed window (ctrl+shift+t), popping
    /// the daemon's recently-closed stack. Reopened windows always
    /// take focus regardless of how the original was opened: the user
    /// pressed a key asking for the page back.
    fn reopen_closed(self: &Rc<Self>) {
        let entry = self.daemon.recently_closed.borrow_mut().pop();
        let Some(entry) = entry else {
            self.flash_bar("nothing to reopen", 1);
            return;
        };
        Self::open(
            &self.daemon,
            Some(entry.url),
            entry.app_id,
            OpenMode::Normal,
        );
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
    /// not consume. Unhandled keys pass through to the page: the URL
    /// prompt opens only via its bound chords (ctrl+l), never from
    /// stray typing. Modified chords are handled by the capture
    /// controller.
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
                    None => glib::Propagation::Proceed,
                }
            }
            // Find mode: entry has focus and eats printable keys; we
            // only see what bubbles past it (Escape/Enter handled in
            // wire_bar on the entry itself). Swallow the rest so keys
            // don't leak into the page under the bar.
            BarMode::Find { .. } | BarMode::Url | BarMode::Palette => glib::Propagation::Proceed,
            BarMode::Confirm { tag } => {
                let Some(answer) = confirm_answer(key, state) else {
                    return glib::Propagation::Proceed;
                };
                self.answer_confirm(&tag, answer);
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
            let weak = Rc::downgrade(self);
            self.bar.entry.connect_changed(move |entry| {
                let Some(this) = weak.upgrade() else { return };
                match this.bar.mode() {
                    BarMode::Find { backwards } => this.run_find(&entry.text(), backwards),
                    BarMode::Palette => this.refresh_palette(&entry.text()),
                    BarMode::Url => this.refresh_completions(&entry.text()),
                    _ => {}
                }
            });
        }
        // Enter: find keeps highlights and returns focus to the page;
        // URL mode navigates.
        {
            let weak = Rc::downgrade(self);
            self.bar.entry.connect_activate(move |entry| {
                let Some(this) = weak.upgrade() else { return };
                match this.bar.mode() {
                    BarMode::Find { .. } => {
                        this.bar.close();
                        this.focus_webview();
                    }
                    BarMode::Url => {
                        // A highlighted completion wins over the typed
                        // text (roadmap H9); plain Enter navigates what
                        // the user typed.
                        let completion = {
                            let state = this.completions.borrow();
                            state
                                .as_ref()
                                .and_then(|s| s.selected.and_then(|i| s.urls.get(i).cloned()))
                        };
                        let text = completion.unwrap_or_else(|| entry.text().trim().to_string());
                        this.completions.replace(None);
                        this.bar.close();
                        if !text.is_empty() {
                            this.navigate(&text);
                        }
                        this.focus_webview();
                    }
                    BarMode::Palette => this.run_palette_selected(),
                    _ => {}
                }
            });
        }
        // Escape inside the entry: cancel find/URL entry entirely.
        // On a bare launcher window, cancelling means the window
        // itself was a mis-fire: close it. Up/Down while the palette
        // is open move its highlight instead of the entry cursor.
        {
            let weak = Rc::downgrade(self);
            let ctrl = gtk::EventControllerKey::new();
            ctrl.connect_key_pressed(move |_, key, _, _| {
                let Some(this) = weak.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                if key == gtk::gdk::Key::Escape {
                    if this.close_if_bare_launcher() {
                        return glib::Propagation::Stop;
                    }
                    this.palette.replace(None);
                    this.completions.replace(None);
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
                if this.bar.mode() == BarMode::Url {
                    match key {
                        gtk::gdk::Key::Down | gtk::gdk::Key::Tab => {
                            this.completions_move(1);
                            return glib::Propagation::Stop;
                        }
                        gtk::gdk::Key::Up | gtk::gdk::Key::ISO_Left_Tab => {
                            this.completions_move(-1);
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

    // ---- URL completion (roadmap H9) --------------------------------

    /// How many history completions the URL bar shows.
    const COMPLETION_ROWS: usize = 6;

    /// Re-query history for the current entry text. Skipped when the
    /// text equals the page's own URL (opening ctrl+l prefills it;
    /// completing against it would just echo the current page).
    fn refresh_completions(self: &Rc<Self>, text: &str) {
        let text = text.trim();
        let current = self.last_url.borrow().clone();
        let hits = if text == current.as_str() {
            Vec::new()
        } else {
            self.daemon.history.complete(text, Self::COMPLETION_ROWS)
        };
        let rows: Vec<(String, String)> = hits
            .iter()
            .map(|h| {
                let title = if h.title.is_empty() {
                    String::new()
                } else {
                    h.title.clone()
                };
                (h.url.clone(), title)
            })
            .collect();
        self.completions.replace(if hits.is_empty() {
            None
        } else {
            Some(CompletionState {
                urls: hits.into_iter().map(|h| h.url).collect(),
                selected: None,
            })
        });
        self.bar.set_completions(&rows, None);
    }

    /// Move the completion highlight (Down/Tab and Up/Shift+Tab).
    /// Wraps through a no-selection state so the typed text is always
    /// reachable again.
    fn completions_move(self: &Rc<Self>, delta: isize) {
        let mut state = self.completions.borrow_mut();
        let Some(state) = state.as_mut() else {
            return;
        };
        let n = state.urls.len() as isize;
        if n == 0 {
            return;
        }
        // Cycle: None -> 0 -> 1 ... n-1 -> None (and reverse).
        let next = match state.selected {
            None if delta > 0 => Some(0),
            None => Some((n - 1) as usize),
            Some(i) => {
                let j = i as isize + delta;
                if j < 0 || j >= n {
                    None
                } else {
                    Some(j as usize)
                }
            }
        };
        state.selected = next;
        self.bar.set_palette_selected(next.unwrap_or(usize::MAX));
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

    /// Navigate this window, normalizing bare hosts the same way the
    /// CLI does (`example.com` -> `https://example.com`).
    fn navigate(self: &Rc<Self>, input: &str) {
        self.restore();
        if let Some(webview) = self.live_webview() {
            let url = crate::ipc_server::normalize_url(input.to_string());
            self.mark_nav_pending(&url);
            self.remember_url(&url);
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
    use super::{
        external_uri_scheme, is_ctrl_click_link, load_was_cancelled, parse_feature_overrides,
        recovery_url, remember_recently_closed, termination_info,
    };

    /// The reopen stack (ctrl+shift+t) records user windows newest
    /// last, skips headless/blank/launcher pages, and caps at 10 so
    /// the key can't dredge up the whole session's history.
    #[test]
    fn reopen_stack_records_user_windows_and_caps() {
        use hwatu_ipc::{OpenMode, WindowInfo};
        let info = |url: &str, mode| WindowInfo {
            id: 1,
            url: url.into(),
            title: "t".into(),
            focused: false,
            suspended: false,
            app_id: None,
            mode,
            web_process_terminated: None,
        };
        let stack = std::cell::RefCell::new(Vec::new());
        // Recorded: a normal page, and a background window too (the
        // reopen key should bring back agent-opened pages the user
        // closed by hand).
        remember_recently_closed(&stack, info("https://a.example/", OpenMode::Normal));
        remember_recently_closed(&stack, info("https://b.example/", OpenMode::Background));
        // Skipped: headless, blank, launcher.
        remember_recently_closed(&stack, info("https://c.example/", OpenMode::Headless));
        remember_recently_closed(&stack, info("", OpenMode::Normal));
        remember_recently_closed(&stack, info("about:blank", OpenMode::Normal));
        remember_recently_closed(&stack, info(crate::launcher::URI, OpenMode::Normal));
        {
            let closed = stack.borrow();
            assert_eq!(closed.len(), 2);
            // Newest last: pop() must return the most recent close.
            assert_eq!(closed[0].url, "https://a.example/");
            assert_eq!(closed[1].url, "https://b.example/");
        }
        // Cap at 10, evicting oldest first: 2 seeds + 12 pushes = 14,
        // so a/b and n0/n1 fall off and n2 is the oldest survivor.
        for i in 0..12 {
            remember_recently_closed(
                &stack,
                info(&format!("https://n{i}.example/"), OpenMode::Normal),
            );
        }
        let closed = stack.borrow();
        assert_eq!(closed.len(), 10);
        assert_eq!(closed[0].url, "https://n2.example/");
        assert_eq!(closed[9].url, "https://n11.example/");
    }

    #[test]
    fn baseline_enables_the_complete_opfs_feature_bundle() {
        for identifier in [
            "StorageAPI",
            "StorageAPIEstimate",
            "FileSystem",
            "FileSystemWritableStream",
        ] {
            assert_eq!(
                super::BASELINE_FEATURE_OVERRIDES
                    .iter()
                    .find(|(name, _)| *name == identifier),
                Some(&(identifier, true)),
                "{identifier} must be enabled for navigator.storage.getDirectory()"
            );
        }
    }

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

    /// MediaStream stays off unless the env opt-in is exact: the
    /// default guards against the enumerateDevices() main-thread wedge
    /// (see apply_view_settings), so a sloppy value must not enable it.
    #[test]
    fn media_stream_env_gate() {
        std::env::remove_var("HWATU_MEDIA_STREAM");
        assert!(!super::media_stream_enabled());
        std::env::set_var("HWATU_MEDIA_STREAM", "1");
        assert!(super::media_stream_enabled());
        std::env::set_var("HWATU_MEDIA_STREAM", "on");
        assert!(super::media_stream_enabled());
        std::env::set_var("HWATU_MEDIA_STREAM", "0");
        assert!(!super::media_stream_enabled());
        std::env::set_var("HWATU_MEDIA_STREAM", "yes");
        assert!(!super::media_stream_enabled());
        std::env::remove_var("HWATU_MEDIA_STREAM");
    }

    #[test]
    fn preferred_width_uses_the_configured_fraction_of_the_viewport() {
        assert_eq!(super::preferred_width(1920, 1.0 / 3.0), 640);
        assert_eq!(super::preferred_width(1366, 1.0 / 3.0), 455);
        assert_eq!(super::preferred_width(1920, 0.25), 480);
    }

    #[test]
    fn preferred_width_defaults_to_one_half() {
        assert_eq!(super::DEFAULT_PREFERRED_WIDTH, 0.5);
        assert_eq!(
            super::preferred_width(1920, super::DEFAULT_PREFERRED_WIDTH),
            960
        );
    }

    #[test]
    fn preferred_width_never_returns_zero() {
        assert_eq!(super::preferred_width(0, super::DEFAULT_PREFERRED_WIDTH), 1);
        assert_eq!(
            super::preferred_width(-1, super::DEFAULT_PREFERRED_WIDTH),
            1
        );
    }

    #[test]
    fn ratio_from_value_accepts_only_fractions_in_zero_one() {
        use serde_json::json;
        assert_eq!(super::ratio_from_value(&json!(0.25)), Some(0.25));
        assert_eq!(super::ratio_from_value(&json!(1.0)), Some(1.0));
        assert_eq!(super::ratio_from_value(&json!(0.0)), None);
        assert_eq!(super::ratio_from_value(&json!(-0.5)), None);
        assert_eq!(super::ratio_from_value(&json!(1.5)), None);
        assert_eq!(super::ratio_from_value(&json!("0.25")), None);
        assert_eq!(super::ratio_from_value(&json!(null)), None);
        assert_eq!(super::ratio_from_value(&json!({})), None);
    }

    #[test]
    fn confirmation_keys_only_claim_explicit_answers() {
        let none = gtk::gdk::ModifierType::empty();
        assert_eq!(super::confirm_answer(gtk::gdk::Key::y, none), Some(true));
        assert_eq!(
            super::confirm_answer(gtk::gdk::Key::Y, gtk::gdk::ModifierType::SHIFT_MASK),
            Some(true)
        );
        assert_eq!(super::confirm_answer(gtk::gdk::Key::n, none), Some(false));
        assert_eq!(super::confirm_answer(gtk::gdk::Key::N, none), Some(false));
        assert_eq!(
            super::confirm_answer(gtk::gdk::Key::Escape, none),
            Some(false)
        );
        assert_eq!(super::confirm_answer(gtk::gdk::Key::a, none), None);
        assert_eq!(super::confirm_answer(gtk::gdk::Key::exclam, none), None);
        assert_eq!(
            super::confirm_answer(gtk::gdk::Key::y, gtk::gdk::ModifierType::CONTROL_MASK),
            None
        );
        assert_eq!(
            super::confirm_answer(gtk::gdk::Key::n, gtk::gdk::ModifierType::ALT_MASK),
            None
        );
    }

    #[test]
    fn cancelled_webkit_load_is_not_a_page_failure() {
        let error = glib::Error::new(webkit6::NetworkError::Cancelled, "Load request cancelled");
        assert!(load_was_cancelled(&error));
    }

    #[test]
    fn cancelled_gio_load_is_not_a_page_failure() {
        let error = glib::Error::new(gtk::gio::IOErrorEnum::Cancelled, "Operation cancelled");
        assert!(load_was_cancelled(&error));
    }

    #[test]
    fn genuine_network_error_remains_a_page_failure() {
        let error = glib::Error::new(webkit6::NetworkError::Failed, "Connection failed");
        assert!(!load_was_cancelled(&error));
    }

    #[test]
    fn only_ctrl_primary_link_clicks_open_new_tabs() {
        let ctrl = gtk::gdk::ModifierType::CONTROL_MASK.bits();
        let shift = gtk::gdk::ModifierType::SHIFT_MASK.bits();

        assert!(is_ctrl_click_link(
            webkit6::NavigationType::LinkClicked,
            1,
            ctrl
        ));
        assert!(is_ctrl_click_link(
            webkit6::NavigationType::LinkClicked,
            1,
            ctrl | shift
        ));
        assert!(!is_ctrl_click_link(
            webkit6::NavigationType::LinkClicked,
            1,
            shift
        ));
        assert!(!is_ctrl_click_link(
            webkit6::NavigationType::LinkClicked,
            3,
            ctrl
        ));
        assert!(!is_ctrl_click_link(webkit6::NavigationType::Other, 1, ctrl));
    }

    #[test]
    fn native_app_schemes_are_external_but_browser_schemes_are_not() {
        assert_eq!(
            external_uri_scheme("zoommtg://zoom.us/join"),
            Some("zoommtg".into())
        );
        assert_eq!(
            external_uri_scheme("mailto:person@example.com"),
            Some("mailto".into())
        );
        assert_eq!(
            external_uri_scheme("ZOOMMTG://zoom.us/join"),
            Some("zoommtg".into())
        );
        assert_eq!(
            external_uri_scheme("web+notes:42"),
            Some("web+notes".into())
        );

        assert_eq!(external_uri_scheme("https://zoom.us/join"), None);
        assert_eq!(external_uri_scheme("HTTP://example.com"), None);
        assert_eq!(external_uri_scheme("javascript:alert(1)"), None);
        assert_eq!(external_uri_scheme("hwatu://launcher"), None);
        assert_eq!(external_uri_scheme("not a uri"), None);
        assert_eq!(external_uri_scheme("1invalid:value"), None);
    }

    #[test]
    fn recovery_url_prefers_live_and_falls_back_to_last_non_empty_url() {
        assert_eq!(
            recovery_url(Some("https://live.test/"), "https://last.test/"),
            Some("https://live.test/".into())
        );
        assert_eq!(
            recovery_url(Some(""), "https://last.test/"),
            Some("https://last.test/".into())
        );
        assert_eq!(recovery_url(None, ""), None);
    }

    #[test]
    fn web_process_termination_reasons_are_stable_and_keep_url() {
        let url = "https://example.test/sign-up".to_string();
        let crashed = termination_info(webkit6::WebProcessTerminationReason::Crashed, url.clone());
        let oom = termination_info(
            webkit6::WebProcessTerminationReason::ExceededMemoryLimit,
            url.clone(),
        );

        assert_eq!(crashed.reason, "crashed");
        assert_eq!(crashed.message, "crashed");
        assert_eq!(crashed.url, url);
        assert_eq!(oom.reason, "oom");
        assert_eq!(oom.message, "was killed (out of memory)");
    }
}
