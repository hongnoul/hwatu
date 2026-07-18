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
        }
    }
    pub fn window(w: WindowInfo) -> Self {
        Response::Ok {
            window: Some(w),
            windows: None,
            adblock: None,
        }
    }
    pub fn windows(ws: Vec<WindowInfo>) -> Self {
        Response::Ok {
            window: None,
            windows: Some(ws),
            adblock: None,
        }
    }
    pub fn adblock(status: AdblockStatus) -> Self {
        Response::Ok {
            window: None,
            windows: None,
            adblock: Some(status),
        }
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
    /// True when the window's WebView has been discarded to save RAM;
    /// it restores automatically on focus.
    #[serde(default)]
    pub suspended: bool,
}
