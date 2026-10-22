// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Justin Hong
//! Downloads: no dialogs, no manager. Every download goes straight to
//! the download directory; progress and completion are emitted as `notice` events.
//!
//! Directory resolution (first hit wins):
//!   1. `HWATU_DOWNLOAD_DIR`
//!   2. XDG user dir (`~/.config/user-dirs.dirs` `XDG_DOWNLOAD_DIR`)
//!   3. `~/Downloads`
//!
//! Name collisions get a ` (n)` suffix rather than overwriting.

use crate::Daemon;
use gtk::prelude::*;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use webkit6::prelude::*;

/// One download's lifecycle record (coverage C5). Kept after
/// completion so `hwatu downloads` reports history for this daemon
/// session, bounded by [`REGISTRY_CAP`].
#[derive(Clone, Debug)]
pub struct DownloadRecord {
    pub id: u64,
    pub url: String,
    pub destination: Option<String>,
    /// "active" | "finished" | "failed"
    pub state: &'static str,
    pub error: Option<String>,
    pub window: Option<u64>,
    pub started_at: std::time::SystemTime,
}

/// Bounded registry of downloads this daemon session started.
#[derive(Default)]
pub struct Registry {
    entries: std::cell::RefCell<Vec<DownloadRecord>>,
    next_id: std::cell::Cell<u64>,
}

const REGISTRY_CAP: usize = 200;

impl Registry {
    fn insert(&self, mut record: DownloadRecord) -> u64 {
        let id = self.next_id.get() + 1;
        self.next_id.set(id);
        record.id = id;
        let mut entries = self.entries.borrow_mut();
        if entries.len() >= REGISTRY_CAP {
            entries.remove(0);
        }
        entries.push(record);
        id
    }

    fn update(&self, id: u64, f: impl FnOnce(&mut DownloadRecord)) {
        if let Some(entry) = self.entries.borrow_mut().iter_mut().find(|e| e.id == id) {
            f(entry);
        }
    }

    /// Newest-last list of the most recent `limit` records.
    pub fn list(&self, limit: Option<usize>) -> Vec<DownloadRecord> {
        let entries = self.entries.borrow();
        let n = limit.unwrap_or(entries.len()).min(entries.len());
        entries[entries.len() - n..].to_vec()
    }
}

pub fn download_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("HWATU_DOWNLOAD_DIR") {
        let dir = dir.trim();
        if !dir.is_empty() {
            return expand_home(dir);
        }
    }
    if let Some(dir) = xdg_download_dir() {
        return dir;
    }
    home().join("Downloads")
}

fn home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}

fn expand_home(raw: &str) -> PathBuf {
    match raw.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None => PathBuf::from(raw),
    }
}

/// Parse `XDG_DOWNLOAD_DIR="$HOME/..."` from user-dirs.dirs; the file
/// format is a fixed shell subset per the xdg-user-dirs spec.
fn xdg_download_dir() -> Option<PathBuf> {
    let config = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home().join(".config"));
    let text = std::fs::read_to_string(config.join("user-dirs.dirs")).ok()?;
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("XDG_DOWNLOAD_DIR=") else {
            continue;
        };
        let value = rest.trim_matches('"');
        let path = if let Some(rel) = value.strip_prefix("$HOME/") {
            home().join(rel)
        } else if value == "$HOME/" || value == "$HOME" {
            // Disabled per spec; fall through to default.
            return None;
        } else {
            PathBuf::from(value)
        };
        return Some(path);
    }
    None
}

/// `report.pdf` -> `report (1).pdf` until the name is free.
fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), String::new()),
    };
    for n in 1u32.. {
        let candidate = dir.join(format!("{stem} ({n}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}

/// Hook download handling onto a WebView's network session. Idempotent
/// per session (WebKit shares the default session across views).
pub fn wire_session(daemon: &Rc<Daemon>, webview: &webkit6::WebView) {
    let Some(session) = webview.network_session() else {
        return;
    };
    unsafe {
        if session.data::<bool>("hwatu-downloads").is_some() {
            return;
        }
        session.set_data("hwatu-downloads", true);
    }
    let daemon = daemon.clone();
    session.connect_download_started(move |_, download| {
        wire_download(&daemon, download);
    });
}

fn wire_download(daemon: &Rc<Daemon>, download: &webkit6::Download) {
    let record_id = daemon.downloads.insert(DownloadRecord {
        id: 0,
        url: download
            .request()
            .and_then(|r| r.uri())
            .map(|u| u.to_string())
            .unwrap_or_default(),
        destination: None,
        state: "active",
        error: None,
        window: owner_window_id(daemon, download),
        started_at: std::time::SystemTime::now(),
    });
    // Pick the destination ourselves; never prompt.
    let registry_daemon = daemon.clone();
    download.connect_decide_destination(move |download, suggested| {
        let dir = download_dir();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            eprintln!("hwatud: cannot create download dir {}: {e}", dir.display());
            download.cancel();
            return true;
        }
        let name = if suggested.is_empty() {
            "download"
        } else {
            suggested
        };
        let dest = unique_path(&dir, name);
        download.set_destination(&dest.display().to_string());
        registry_daemon.downloads.update(record_id, |r| {
            r.destination = Some(dest.display().to_string())
        });
        true
    });

    {
        let daemon = daemon.clone();
        download.connect_finished(move |download| {
            let dest = download
                .destination()
                .map(|d| d.to_string())
                .unwrap_or_default();
            daemon.downloads.update(record_id, |r| {
                if r.state == "active" {
                    r.state = "finished";
                }
                r.destination = Some(dest.clone());
            });
            daemon.events.emit(
                "download",
                owner_window_id(&daemon, download),
                serde_json::json!({ "state": "finished", "path": dest }),
            );
            flash_on_owner(&daemon, download, &format!("saved {dest}"), 5);
        });
    }
    {
        let daemon = daemon.clone();
        download.connect_failed(move |download, error| {
            daemon.downloads.update(record_id, |r| {
                r.state = "failed";
                r.error = Some(error.to_string());
            });
            daemon.events.emit(
                "download",
                owner_window_id(&daemon, download),
                serde_json::json!({ "state": "failed", "error": error.to_string() }),
            );
            // Cancel also lands here; stay quiet about user cancels.
            flash_on_owner(&daemon, download, &format!("download failed: {error}"), 8);
        });
    }
}

/// The id of the window whose WebView started this download, if that
/// window is still open.
fn owner_window_id(daemon: &Rc<Daemon>, download: &webkit6::Download) -> Option<u64> {
    let origin = download.web_view()?;
    daemon
        .windows
        .borrow()
        .values()
        .find(|w| w.live_webview().is_some_and(|wv| wv == origin))
        .map(|w| w.id)
}

/// Emit a notice for the window that started the download,
/// falling back to any live window (the origin may already be closed).
fn flash_on_owner(daemon: &Rc<Daemon>, download: &webkit6::Download, message: &str, secs: u64) {
    let origin = download.web_view();
    let windows = daemon.windows.borrow();
    let target = windows
        .values()
        .find(|w| match (&origin, w.live_webview()) {
            (Some(o), Some(wv)) => wv == *o,
            _ => false,
        })
        .or_else(|| windows.values().next());
    if let Some(win) = target {
        win.notice(message, secs);
    } else {
        println!("hwatud: {message}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_path_appends_counter() {
        let dir = std::env::temp_dir().join(format!("hwatu-dl-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(unique_path(&dir, "a.txt"), dir.join("a.txt"));
        std::fs::write(dir.join("a.txt"), b"x").unwrap();
        assert_eq!(unique_path(&dir, "a.txt"), dir.join("a (1).txt"));
        std::fs::write(dir.join("a (1).txt"), b"x").unwrap();
        assert_eq!(unique_path(&dir, "a.txt"), dir.join("a (2).txt"));
        // No-extension and dotfile cases.
        assert_eq!(unique_path(&dir, "README"), dir.join("README"));
        std::fs::write(dir.join("README"), b"x").unwrap();
        assert_eq!(unique_path(&dir, "README"), dir.join("README (1)"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn expand_home_tilde() {
        let h = home();
        assert_eq!(expand_home("~/x"), h.join("x"));
        assert_eq!(expand_home("/abs/x"), PathBuf::from("/abs/x"));
    }
}
