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
    /// Capture the visible viewport as a PNG. Writes to `path` (or a
    /// temp file) and returns the file path in [`Response::Ok::path`].
    Screenshot {
        #[serde(default)]
        id: Option<u64>,
        #[serde(default)]
        path: Option<String>,
    },
    /// Block until the window finishes loading (or `timeout_ms`).
    WaitLoad {
        #[serde(default)]
        id: Option<u64>,
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
}

fn default_true() -> bool {
    true
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
}
