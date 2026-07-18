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
//! - Restored windows point at each page's last known URL. Scroll
//!   position, form state, and per-tab history die with the web
//!   process; a URL is what can honestly be brought back.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEntry {
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
}

/// Versioned envelope so a future format change can migrate instead of
/// silently misparsing.
#[derive(Debug, Serialize, Deserialize)]
struct SessionFile {
    version: u32,
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
    let _ = std::fs::remove_file(&path);
    match serde_json::from_slice::<SessionFile>(&bytes) {
        Ok(file) if file.version == VERSION => file
            .windows
            .into_iter()
            .filter(|w| !w.url.is_empty())
            .collect(),
        Ok(file) => {
            eprintln!(
                "hwatud: session file version {} unsupported; ignoring",
                file.version
            );
            Vec::new()
        }
        Err(e) => {
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
            },
            SessionEntry {
                url: "https://rust-lang.org/".into(),
                title: String::new(),
                app_id: None,
            },
        ];
        save(&entries);
        let restored = take();
        assert_eq!(restored.len(), 2);
        assert_eq!(restored[0].url, "https://example.com/");
        assert_eq!(restored[0].app_id.as_deref(), Some("mail"));
        assert_eq!(restored[1].url, "https://rust-lang.org/");

        // take() consumes: a second call restores nothing (no loops on
        // a session that crashes the daemon during restore).
        assert!(take().is_empty());

        // Entries with empty URLs are dropped on read.
        save(&[SessionEntry {
            url: String::new(),
            title: "blank".into(),
            app_id: None,
        }]);
        assert!(take().is_empty());

        // A corrupt file is ignored, not fatal.
        let path = dir.join("hwatu").join("session.json");
        std::fs::write(&path, b"not json").unwrap();
        assert!(take().is_empty());

        // A future version is ignored, not misparsed.
        std::fs::write(&path, br#"{"version": 999, "windows": []}"#).unwrap();
        assert!(take().is_empty());

        // clear() removes the file.
        save(&entries);
        clear();
        assert!(!path.exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
