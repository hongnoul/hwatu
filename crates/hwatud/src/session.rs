//! Crash resilience: the daemon owns every window, so a daemon crash
//! (or OOM kill, or logout) used to take the whole browsing session
//! with it. This module persists the open-window set to disk and
//! restores it on the next daemon start.
//!
//! Mechanics:
//! - The window registry (url, title, app_id per window) is serialized
//!   to `$XDG_STATE_HOME/hwatu/session.json` (`~/.local/state/...`),
//!   debounced so navigation-heavy pages don't thrash the disk.
//! - A clean `hwatu quit` deletes the file: intentional exits do not
//!   restore. Anything else (crash, SIGKILL, compositor logout) leaves
//!   the file behind, and the next `hwatud` reopens the windows.
//! - Restored windows point at each page's last known URL and reopen
//!   in their original mode (normal/background); scroll position,
//!   form state, and per-tab history die with the web process; a URL
//!   is what can honestly be brought back.

use hwatu_ipc::OpenMode;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEntry {
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    /// How the window was opened. Restore must reopen with the same
    /// mode: promoting an agent's background window to a focused
    /// Normal one after a crash would steal the user's focus. Absent
    /// in v1 files written before this field existed; those were all
    /// effectively user windows, so Normal is the honest default.
    #[serde(default)]
    pub mode: OpenMode,
}

/// Versioned envelope so a future format change can migrate instead of
/// silently misparsing.
#[derive(Debug, Serialize, Deserialize)]
struct SessionFile {
    version: u32,
    /// Socket path of the daemon that wrote the snapshot. A daemon on
    /// a different socket (test harness, second instance with its own
    /// XDG_RUNTIME_DIR) must not consume another daemon's crash
    /// snapshot: doing so "restores" windows the original daemon still
    /// owns, popping them onto the user's workspace. Absent in files
    /// written before this field existed; those belonged to the
    /// default daemon, so absence matches any reader.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    socket: Option<String>,
    windows: Vec<SessionEntry>,
}

const VERSION: u32 = 1;

/// `$XDG_STATE_HOME/hwatu/session.json`, falling back to
/// `~/.local/state/hwatu/session.json`. State (not cache): losing it
/// loses user data, so it must survive cache cleaners.
fn session_file() -> Option<PathBuf> {
    let base = std::env::var("XDG_STATE_HOME")
        .ok()
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".local").join("state"))
        })?;
    Some(base.join("hwatu").join("session.json"))
}

/// Write the current window set. Atomic (write to a temp file, then
/// rename) so a crash mid-write cannot corrupt the previous snapshot.
pub fn save(entries: &[SessionEntry]) {
    let Some(path) = session_file() else { return };
    let Some(dir) = path.parent() else { return };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let file = SessionFile {
        version: VERSION,
        socket: Some(hwatu_ipc::socket_path().to_string_lossy().into_owned()),
        windows: entries.to_vec(),
    };
    let Ok(json) = serde_json::to_vec_pretty(&file) else {
        return;
    };
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, json).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// Read and consume the crash leftovers. The file is removed
/// immediately so a daemon that crashes *during* restore does not
/// loop on the same session forever.
pub fn take() -> Vec<SessionEntry> {
    let Some(path) = session_file() else {
        return Vec::new();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return Vec::new();
    };
    match serde_json::from_slice::<SessionFile>(&bytes) {
        Ok(file) if file.version == VERSION => {
            let ours = hwatu_ipc::socket_path().to_string_lossy().into_owned();
            if file.socket.as_ref().is_some_and(|s| *s != ours) {
                // Another daemon's snapshot (different socket). Leave
                // the file for its owner; restore nothing here.
                return Vec::new();
            }
            let _ = std::fs::remove_file(&path);
            file.windows
                .into_iter()
                .filter(|w| !w.url.is_empty())
                .collect()
        }
        Ok(file) => {
            let _ = std::fs::remove_file(&path);
            eprintln!(
                "hwatud: session file version {} unsupported; ignoring",
                file.version
            );
            Vec::new()
        }
        Err(e) => {
            let _ = std::fs::remove_file(&path);
            eprintln!("hwatud: unreadable session file ({e}); ignoring");
            Vec::new()
        }
    }
}

/// Remove the session file: called on clean quit so an intentional
/// exit does not resurrect windows.
pub fn clear() {
    if let Some(path) = session_file() {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One test owns the whole lifecycle: `session_file` reads
    /// XDG_STATE_HOME from the environment, and parallel tests
    /// mutating process env race, so keep it to a single #[test].
    #[test]
    fn save_take_clear_roundtrip() {
        let dir = std::env::temp_dir().join(format!("hwatu-session-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("XDG_STATE_HOME", &dir);

        // Nothing saved yet: nothing to take.
        assert!(take().is_empty());

        let entries = vec![
            SessionEntry {
                url: "https://example.com/".into(),
                title: "Example".into(),
                app_id: Some("mail".into()),
                mode: OpenMode::Normal,
            },
            SessionEntry {
                url: "https://rust-lang.org/".into(),
                title: String::new(),
                app_id: None,
                mode: OpenMode::Background,
            },
        ];
        save(&entries);
        let restored = take();
        assert_eq!(restored.len(), 2);
        assert_eq!(restored[0].url, "https://example.com/");
        assert_eq!(restored[0].app_id.as_deref(), Some("mail"));
        assert_eq!(restored[0].mode, OpenMode::Normal);
        assert_eq!(restored[1].url, "https://rust-lang.org/");
        // Modes round-trip: a background window must not be promoted
        // to a focused Normal window by a crash restore.
        assert_eq!(restored[1].mode, OpenMode::Background);

        // A pre-mode v1 file (no "mode" field) defaults to Normal.
        let path = dir.join("hwatu").join("session.json");
        std::fs::write(
            &path,
            br#"{"version": 1, "windows": [{"url": "https://old.example/"}]}"#,
        )
        .unwrap();
        let legacy = take();
        assert_eq!(legacy.len(), 1);
        assert_eq!(legacy[0].mode, OpenMode::Normal);

        // take() consumes: a second call restores nothing (no loops on
        // a session that crashes the daemon during restore).
        assert!(take().is_empty());

        // Entries with empty URLs are dropped on read.
        save(&[SessionEntry {
            url: String::new(),
            title: "blank".into(),
            app_id: None,
            mode: OpenMode::Normal,
        }]);
        assert!(take().is_empty());

        // A corrupt file is ignored, not fatal.
        std::fs::write(&path, b"not json").unwrap();
        assert!(take().is_empty());

        // A future version is ignored, not misparsed.
        std::fs::write(&path, br#"{"version": 999, "windows": []}"#).unwrap();
        assert!(take().is_empty());

        // Another daemon's snapshot (different socket) is left alone:
        // consuming it would "restore" windows that daemon still owns.
        std::fs::write(
            &path,
            br#"{"version": 1, "socket": "/somewhere/else/hwatu.sock",
                 "windows": [{"url": "https://foreign.example/"}]}"#,
        )
        .unwrap();
        assert!(take().is_empty());
        assert!(path.exists(), "foreign snapshot must not be consumed");
        std::fs::remove_file(&path).unwrap();

        // A pre-socket file (no "socket" field) belongs to the default
        // daemon and is restored by any reader.
        std::fs::write(
            &path,
            br#"{"version": 1, "windows": [{"url": "https://presocket.example/"}]}"#,
        )
        .unwrap();
        assert_eq!(take().len(), 1);

        // clear() removes the file.
        save(&entries);
        clear();
        assert!(!path.exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
