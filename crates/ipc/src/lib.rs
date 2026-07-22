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
    /// response until the load finishes or `timeout_ms` expires.
    Navigate {
        #[serde(default)]
        id: Option<u64>,
        url: String,
        #[serde(default = "default_true")]
        wait: bool,
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
    /// Block until the window finishes loading (or `timeout_ms`).
    WaitLoad {
        #[serde(default)]
        id: Option<u64>,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
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
}

fn default_true() -> bool {
    true
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
