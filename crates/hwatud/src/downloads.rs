//! Downloads: no dialogs, no manager. Every download goes straight to
//! the download directory; the bar flashes progress and completion.
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
    std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/tmp"))
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
    // Pick the destination ourselves; never prompt.
    download.connect_decide_destination(move |download, suggested| {
        let dir = download_dir();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            eprintln!("hwatud: cannot create download dir {}: {e}", dir.display());
            download.cancel();
            return true;
        }
        let name = if suggested.is_empty() { "download" } else { suggested };
        let dest = unique_path(&dir, name);
        download.set_destination(&dest.display().to_string());
        true
    });

    {
        let daemon = daemon.clone();
        download.connect_finished(move |download| {
            let dest = download
                .destination()
                .map(|d| d.to_string())
                .unwrap_or_default();
            flash_on_owner(&daemon, download, &format!("saved {dest}"), 5);
        });
    }
    {
        let daemon = daemon.clone();
        download.connect_failed(move |download, error| {
            // Cancel also lands here; stay quiet about user cancels.
            flash_on_owner(&daemon, download, &format!("download failed: {error}"), 8);
        });
    }
}

/// Flash a message on the bar of the window that started the download,
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
        win.flash_bar(message, secs);
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
