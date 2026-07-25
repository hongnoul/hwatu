// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! Wire protocol between the `hana` client and the `hwatud` daemon.
//!
//! Newline-delimited JSON over a Unix domain socket. One request per
//! connection for the MVP: connect, send a [`Request`], read a
//! [`Response`], disconnect.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Resolve the daemon socket path: `$XDG_RUNTIME_DIR/hwatu.sock`,
/// falling back to `/tmp/hwatu-$UID.sock`.
pub fn socket_path() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(dir).join("hwatu.sock");
    }
    let uid = unsafe { libc_geteuid() };
    PathBuf::from(format!("/tmp/hwatu-{uid}.sock"))
}

// Tiny FFI shim so the client stays dependency-free.
extern "C" {
    #[link_name = "geteuid"]
    fn libc_geteuid() -> u32;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    /// Open a new browser window. The daemon adopts a prewarmed WebView,
    /// so this returns as soon as the window is mapped.
    Open {
        url: Option<String>,
        /// Wayland app_id / X11 WM_CLASS for tiling-WM window rules.
        #[serde(default)]
        app_id: Option<String>,
        /// How the window is shown; see [`OpenMode`]. Absent on the
        /// wire means [`OpenMode::Normal`], so old clients keep working.
        #[serde(default)]
        mode: OpenMode,
    },
    /// List open windows.
    List,
    /// Close a window by id.
    Close { id: u64 },
    /// Control the built-in content blocker.
    Adblock { action: AdblockCmd },
    /// Ask the daemon to exit.
    Quit,
    /// Health check / used by the client to detect a live daemon.
    Ping,

    // ---- automation (agent integration) ----------------------------
    // These primitives let coding agents (e.g. jcode) drive a window:
    // run JS in the page, navigate, screenshot, and wait for loads.
    // `id: None` targets the focused window, else the only window.
    /// Run JavaScript in a window's page. `js` is a *function body*
    /// (so `return` works), and a returned Promise is awaited. The
    /// result comes back as JSON in [`Response::Ok::value`].
    Eval {
        #[serde(default)]
        id: Option<u64>,
        js: String,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Navigate an existing window. `wait` (default true) blocks the
    /// response until the load reaches `until` (default: settled) or
    /// `timeout_ms` expires.
    Navigate {
        #[serde(default)]
        id: Option<u64>,
        url: String,
        #[serde(default = "default_true")]
        wait: bool,
        /// How far the load must progress before the reply (see
        /// [`LoadStage`]). Absent on the wire means `Settled`, so old
        /// clients keep the full-load semantics they were built for.
        #[serde(default)]
        until: LoadStage,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Capture the page as a PNG. Writes to `path` (or a temp file)
    /// and returns the file path in [`Response::Ok::path`].
    Screenshot {
        #[serde(default)]
        id: Option<u64>,
        #[serde(default)]
        path: Option<String>,
        /// Capture the entire document instead of just the visible
        /// viewport. Agents hunting for below-the-fold content should
        /// use this instead of scroll-and-shoot loops.
        #[serde(default)]
        full: bool,
    },
    /// Block until the window's load reaches `until` (default:
    /// settled) or `timeout_ms` expires.
    WaitLoad {
        #[serde(default)]
        id: Option<u64>,
        #[serde(default)]
        until: LoadStage,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// One-roundtrip verification pass: open a headless window, load
    /// `url`, wait for `until`, optionally run JS and take a
    /// screenshot, then close the window (unless `keep`). Replies with
    /// everything at once in [`Response::Ok::value`]: final url,
    /// title, eval result, screenshot path, console entries, timings.
    /// Collapses the open/wait/eval/shot/close agent loop (5 process
    /// spawns + 5 socket roundtrips) into one.
    Check {
        url: String,
        /// JS to run once the load reaches `until` (expression or
        /// function body, same semantics as [`Request::Eval`]).
        #[serde(default)]
        eval: Option<String>,
        /// Take a screenshot (to a temp file unless `shot_path`).
        #[serde(default)]
        shot: bool,
        /// Screenshot destination; implies `shot`.
        #[serde(default)]
        shot_path: Option<String>,
        /// Screenshot the full document instead of the viewport.
        #[serde(default)]
        full: bool,
        /// Pixel-diff the loaded page against this baseline PNG and
        /// include the score/regions in the reply (same output as
        /// [`Request::Diff`]). Folds the verify loop's pixel tier into
        /// the one roundtrip: `--eval` answers "is the DOM right",
        /// `--baseline` answers "does it look right".
        #[serde(default)]
        baseline: Option<String>,
        /// Per-channel diff tolerance 0-255 (default 8); only with `baseline`.
        #[serde(default)]
        tolerance: Option<u8>,
        /// Write a mismatch heatmap PNG here; only with `baseline`.
        #[serde(default)]
        heatmap: Option<String>,
        /// Load stage gating eval/shot (default: settled).
        #[serde(default)]
        until: LoadStage,
        /// Keep the window open and report its id instead of closing.
        #[serde(default)]
        keep: bool,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Speculatively start loading `url` in a parked headless window,
    /// replying immediately. The next [`Request::Check`] of the same
    /// URL adopts the window instead of navigating a fresh one, so the
    /// load happens while the agent is still thinking (or while the
    /// dev server rebuilds) and the check pays ~0 load latency.
    /// Unclaimed prefetches expire after a short TTL. Fire-and-forget:
    /// a prefetch is never required for correctness.
    Prefetch { url: String },
    /// Detect CAPTCHA / anti-bot challenge UI, and optionally wait for a
    /// human to clear it. This does not solve or bypass the challenge; it
    /// returns structured state so an agent can pause/resume safely.
    Challenge {
        #[serde(default)]
        id: Option<u64>,
        /// When true, poll until the challenge disappears or timeout fires.
        #[serde(default)]
        wait: bool,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Present (raise/focus) a window.
    Focus { id: u64 },
    /// Set a `<input type=file>`'s files from a path on disk. The
    /// daemon reads the file and injects it into the page as a `File`
    /// via `DataTransfer`, the standard automation-harness technique
    /// (programmatically clicking the OS picker is blocked by WebKit,
    /// but assigning `input.files` is not).
    Upload {
        #[serde(default)]
        id: Option<u64>,
        selector: String,
        path: String,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Scroll the page and report where it landed. Exactly one way to
    /// say "where": `selector` (scrolled into view, disambiguated by
    /// `nth`/`contains`), `to_y` (absolute pixels), or `by_pages`
    /// (relative viewport-heights; default 1.0 when nothing is given).
    /// The response `value` reports match count, the matched element's
    /// text, and the resulting scroll position, so an agent always
    /// knows what it hit and whether the bottom was reached.
    Scroll {
        #[serde(default)]
        id: Option<u64>,
        /// CSS selector to scroll into view (centered).
        #[serde(default)]
        selector: Option<String>,
        /// 0-based index among selector matches (default 0).
        #[serde(default)]
        nth: Option<u32>,
        /// Keep only selector matches whose text contains this.
        #[serde(default)]
        contains: Option<String>,
        /// Absolute scroll target in CSS pixels.
        #[serde(default)]
        to_y: Option<f64>,
        /// Relative scroll in viewport heights (negative = up).
        #[serde(default)]
        by_pages: Option<f64>,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Token-cheap structured page state: url, title, visible text
    /// (bounded), and an indexed list of interactable elements. The
    /// indices ("refs") are remembered by the page, so a follow-up
    /// [`Request::Click`]/[`Request::Type`] can target `ref: n`
    /// without a selector. The cheap alternative to a screenshot.
    Snapshot {
        #[serde(default)]
        id: Option<u64>,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Click an element: by CSS `selector` (disambiguated by
    /// `nth`/`contains`, like Scroll) or by a `ref` from the last
    /// [`Request::Snapshot`]. Dispatches real pointer/mouse events;
    /// the response reports what was hit (match count, tag, text).
    Click {
        #[serde(default)]
        id: Option<u64>,
        #[serde(default)]
        selector: Option<String>,
        #[serde(default)]
        nth: Option<u32>,
        #[serde(default)]
        contains: Option<String>,
        /// Interactable index from the last snapshot of this window.
        #[serde(default)]
        r#ref: Option<u32>,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Type text into an input/textarea/select/contenteditable,
    /// targeted like [`Request::Click`]. Values are set through the
    /// native setter and followed by `input`/`change` events, so
    /// framework-controlled inputs (React et al.) see the change.
    Type {
        #[serde(default)]
        id: Option<u64>,
        #[serde(default)]
        selector: Option<String>,
        #[serde(default)]
        nth: Option<u32>,
        #[serde(default)]
        contains: Option<String>,
        /// Interactable index from the last snapshot of this window.
        #[serde(default)]
        r#ref: Option<u32>,
        text: String,
        /// Replace the current value (default) instead of appending.
        #[serde(default = "default_true")]
        clear: bool,
        /// Press Enter afterwards (submits the enclosing form if the
        /// page did not handle the keydown itself).
        #[serde(default)]
        enter: bool,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Read the window's console/error/network capture buffer:
    /// console.* calls, uncaught exceptions, unhandled rejections,
    /// failed resource loads, and HTTP >= 400 responses. `clear`
    /// drains what was read, so a verify loop can diff runs.
    Console {
        #[serde(default)]
        id: Option<u64>,
        #[serde(default)]
        clear: bool,
        /// Return at most the last N entries.
        #[serde(default)]
        limit: Option<usize>,
    },
    /// Extract the page's motion spec: every CSS animation, transition
    /// and Web-Animations-API animation as structured JSON (keyframes,
    /// duration, delay, easing, iteration count), plus `@keyframes`
    /// rules and declared-but-idle `transition-*` styles from CSSOM.
    /// Motion is numbers, not pixels: an agent copies/verifies easing
    /// curves exactly instead of eyeballing frames.
    Motion {
        #[serde(default)]
        id: Option<u64>,
        /// Observe the live page instead of only reading declared
        /// animation: sample moving elements per frame for
        /// `observe_ms`, then fit motion models (linear velocity,
        /// loop period, easing curve) in the daemon. Catches
        /// script-driven motion (requestAnimationFrame marquees,
        /// JS tickers) that `getAnimations()` cannot see.
        #[serde(default)]
        observe: bool,
        /// Observation window in ms (default 2500).
        #[serde(default)]
        observe_ms: Option<u64>,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Freeze animation time: pause every animation on the page and
    /// set its currentTime, so the page renders a deterministic frame
    /// for screenshot/diff. `time_ms` seeks to an absolute time;
    /// `progress` (0.0–1.0) seeks each animation proportionally to its
    /// own duration. `resume` unpauses everything instead.
    Seek {
        #[serde(default)]
        id: Option<u64>,
        /// Absolute animation time in ms (applied to every animation).
        #[serde(default)]
        time_ms: Option<f64>,
        /// Per-animation fractional progress (0.0 = start, 1.0 = end).
        #[serde(default)]
        progress: Option<f64>,
        /// Unpause all animations, restoring live playback.
        #[serde(default)]
        resume: bool,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Perceptual pixel diff of two windows (or a window against a
    /// baseline PNG). Returns a match score (percent of pixels within
    /// tolerance), the bounding boxes of the largest mismatched
    /// regions, and optionally writes a heatmap PNG highlighting the
    /// differences. This is the feedback signal that lets an agent
    /// *converge* on pixel-perfect instead of eyeballing screenshots.
    Diff {
        /// First window.
        id: u64,
        /// Second window to compare against...
        #[serde(default)]
        other: Option<u64>,
        /// ...or a baseline PNG on disk (exactly one of the two).
        #[serde(default)]
        baseline: Option<String>,
        /// Per-channel tolerance 0-255 before a pixel counts as
        /// different (default 8, forgiving of AA/compression noise).
        #[serde(default)]
        tolerance: Option<u8>,
        /// Write a heatmap PNG (mismatches in red over a dimmed base)
        /// to this path.
        #[serde(default)]
        heatmap: Option<String>,
        /// Diff the full document instead of the visible viewport.
        #[serde(default)]
        full: bool,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Assert page state, polling until it holds or `timeout_ms`
    /// expires: `selector` matches an element (disambiguated by `nth`/
    /// `contains`), optionally with text containing `text`; `absent`
    /// inverts (assert no match). One command instead of an eval-poll
    /// script; failure reports what WAS found (match count, actual
    /// text), so a failed assertion is directly actionable.
    Expect {
        #[serde(default)]
        id: Option<u64>,
        selector: String,
        #[serde(default)]
        nth: Option<u32>,
        /// Keep only matches whose text contains this (a filter, like
        /// Click's).
        #[serde(default)]
        contains: Option<String>,
        /// Require the matched element's text to contain this. Unlike
        /// `contains`, a `text` mismatch fails the assertion and the
        /// error reports the element's actual text.
        #[serde(default)]
        text: Option<String>,
        /// Assert the selector matches nothing instead.
        #[serde(default)]
        absent: bool,
        /// Require the matched element to be actually visible: nonzero
        /// box, not display:none/visibility:hidden/opacity:0, and not
        /// covered by another element (elementFromPoint at its center
        /// resolves inside it). Catches rendered-but-invisible UI that
        /// a bare existence check false-passes.
        #[serde(default)]
        visible: bool,
        /// Poll deadline (default 5000 ms; 0 = a single check).
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Control a page's *virtual clock*. Where [`Request::Seek`] pins
    /// declarative animations (CSS/WAAPI), Clock also freezes the
    /// clocks script-driven animation reads: `requestAnimationFrame`,
    /// `performance.now`, `Date.now`, zero-argument `Date` construction,
    /// and `setTimeout`/`setInterval` are wrapped at document start behind
    /// one controllable timeline, and `document.getAnimations()` is driven
    /// from the same timeline. A
    /// rAF-driven marquee that Seek cannot touch freezes under
    /// `pause` and advances deterministically under `step`.
    Clock {
        #[serde(default)]
        id: Option<u64>,
        action: ClockAction,
        /// Milliseconds: the amount for `step`, the absolute virtual
        /// time for `set`. Ignored by `pause`/`resume`.
        #[serde(default)]
        ms: Option<f64>,
        /// PRNG seed for `seed`. Ignored by every other action.
        #[serde(default)]
        seed: Option<u64>,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Set a window's viewport size (CSS pixels). For headless windows
    /// this re-allocates the offscreen toplevel; for mapped windows it
    /// resizes the window (compositor policy permitting). The point is
    /// matrix verification: responsive pages must be checked at several
    /// widths, and a resize on a warm window costs milliseconds while a
    /// fresh browser context costs seconds.
    Resize {
        #[serde(default)]
        id: Option<u64>,
        width: i32,
        height: i32,
    },
}

fn default_true() -> bool {
    true
}

/// How far a load must progress before a wait releases. Real pages
/// keep loading subresources (fonts, third-party JS, images) long
/// after the DOM is usable; most agent checks only need the DOM, so
/// waiting for the full settle is paying tail latency for nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadStage {
    /// The new document has replaced the old one (WebKit Committed).
    /// Earliest point where evals target the new page; its DOM may
    /// still be streaming in.
    Committed,
    /// `DOMContentLoaded`: the DOM is fully parsed and queryable.
    /// The right default for snapshot/eval checks on real pages.
    Dom,
    /// Full load finished and no follow-up navigation is pending
    /// (every subresource done). The strongest guarantee, and the
    /// wire default for backward compatibility.
    #[default]
    Settled,
}

impl LoadStage {
    /// Parse a user-facing stage name.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "committed" | "commit" => Some(Self::Committed),
            "dom" | "ready" => Some(Self::Dom),
            "settled" | "load" | "full" => Some(Self::Settled),
            _ => None,
        }
    }
}

/// How an opened window is shown. Built for agent verification flows
/// that must not steal the user's focus.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenMode {
    /// Map and request focus (`present`). What a human asked for.
    #[default]
    Normal,
    /// Map the window but do not request activation: it appears in the
    /// WM layout, renders normally (eval/shot work), and focus stays
    /// where it is. Compositor policy has the final say; pair with a
    /// WM rule on `--app-id` to also keep it off the current workspace.
    Background,
    /// Never map a toplevel. The WebView lives offscreen in a
    /// `gtk::OffscreenHolder`-less window kept unrealized; rendering is
    /// driven by WebKit itself, so eval/goto/upload work and `shot`
    /// captures the page. Invisible to the WM entirely.
    Headless,
}

/// What to do with a page's virtual clock (see [`Request::Clock`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClockAction {
    /// Freeze virtual time. rAF stops firing, timers stop expiring,
    /// `performance.now()`/`Date.now()`/`new Date()` stop advancing, and
    /// running CSS/WAAPI animations are paused at the current virtual time.
    Pause,
    /// Return to real time. Wrapped clocks resume advancing from the
    /// current virtual time (monotonic: no backwards jumps).
    Resume,
    /// Advance a paused clock by `ms` virtual milliseconds: due timers
    /// fire, one rAF batch runs per 16.67 ms tick, and CSS/WAAPI
    /// currentTime advances by the same amount. Deterministic.
    Step,
    /// Pause and set absolute virtual time to `ms` (milliseconds since
    /// the clock was installed). Stepping semantics as `step`, from
    /// the current virtual time; going backwards is an error.
    Set,
    /// Report the clock's state without changing it.
    Status,
    /// Replace `Math.random` with a deterministic PRNG (mulberry32)
    /// seeded from `seed`. Applies immediately to the current page and
    /// persists for future loads in the same window (installed from
    /// document start, before page scripts can capture the native
    /// PRNG). Same seed + same virtual timeline => identical
    /// `Math.random()` sequences across loads. Without `seed`, pages
    /// keep native `Math.random`.
    Seed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdblockCmd {
    /// Enable blocking (persisted to config).
    On,
    /// Disable blocking (persisted to config).
    Off,
    /// Report current state.
    Status,
    /// Re-download filter lists and recompile.
    Update,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Response {
    Ok {
        #[serde(skip_serializing_if = "Option::is_none")]
        window: Option<WindowInfo>,
        #[serde(skip_serializing_if = "Option::is_none")]
        windows: Option<Vec<WindowInfo>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        adblock: Option<AdblockStatus>,
        /// Eval result (JSON), or ping capability info.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<serde_json::Value>,
        /// File path produced by a screenshot.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    Err {
        message: String,
    },
}

impl Response {
    pub fn ok() -> Self {
        Response::Ok {
            window: None,
            windows: None,
            adblock: None,
            value: None,
            path: None,
        }
    }
    pub fn window(w: WindowInfo) -> Self {
        let mut r = Response::ok();
        if let Response::Ok { window, .. } = &mut r {
            *window = Some(w);
        }
        r
    }
    pub fn windows(ws: Vec<WindowInfo>) -> Self {
        let mut r = Response::ok();
        if let Response::Ok { windows, .. } = &mut r {
            *windows = Some(ws);
        }
        r
    }
    pub fn adblock(status: AdblockStatus) -> Self {
        let mut r = Response::ok();
        if let Response::Ok { adblock, .. } = &mut r {
            *adblock = Some(status);
        }
        r
    }
    pub fn value(v: serde_json::Value) -> Self {
        let mut r = Response::ok();
        if let Response::Ok { value, .. } = &mut r {
            *value = Some(v);
        }
        r
    }
    pub fn path(p: impl Into<String>) -> Self {
        let mut r = Response::ok();
        if let Response::Ok { path, .. } = &mut r {
            *path = Some(p.into());
        }
        r
    }
    pub fn err(message: impl Into<String>) -> Self {
        Response::Err {
            message: message.into(),
        }
    }
}

/// State of the built-in content blocker, as reported by the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdblockStatus {
    pub enabled: bool,
    /// Compiled content-blocker rules currently active.
    pub rules: usize,
    /// Human-readable description of the filter source.
    pub source: String,
    /// True while WebKit is compiling a ruleset.
    pub compiling: bool,
    /// True while filter lists are being re-downloaded.
    pub updating: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowInfo {
    pub id: u64,
    pub url: String,
    pub title: String,
    /// True when this window currently has WM focus. Lets automation
    /// clients find "the window the user is looking at".
    #[serde(default)]
    pub focused: bool,
    /// True when the window's WebView has been discarded to save RAM;
    /// it restores automatically on focus.
    #[serde(default)]
    pub suspended: bool,
    /// Wayland app_id the window was opened with (`--app-id`), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    /// How the window was opened (normal/background/headless). `focus`
    /// promotes a window to normal.
    #[serde(default, skip_serializing_if = "is_normal")]
    pub mode: OpenMode,
}

fn is_normal(mode: &OpenMode) -> bool {
    *mode == OpenMode::Normal
}
