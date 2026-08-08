// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! Persistent per-site decisions (roadmap H5): permission grants and
//! per-site zoom, one JSON file in the XDG data dir.
//!
//! Before this store, permission decisions lived in daemon-lifetime
//! RAM: every restart re-asked the same mic/cam/notification
//! questions, which is exactly the nagging the memory existed to
//! stop. Zoom shares the store because it is the same shape of state
//! (host -> preference) with the same lifecycle.
//!
//! Ephemeral-profile daemons never touch disk: the store degrades to
//! the old RAM-only behavior.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Data {
    /// "host:kind" -> allow. (Key shape matches the old prompt-memory
    /// keys minus the "perm:" prefix.)
    #[serde(default)]
    permissions: HashMap<String, bool>,
    /// host -> zoom level (1.0 = 100%). Only non-default levels are
    /// stored; resetting to 100% removes the entry.
    #[serde(default)]
    zoom: HashMap<String, f64>,
}

/// Daemon-wide store, shared by all windows. Single-threaded (GTK
/// main thread only), like the rest of the daemon state.
pub struct SiteStore {
    /// None = ephemeral mode (RAM only).
    path: Option<PathBuf>,
    data: RefCell<Data>,
}

pub type Store = Rc<SiteStore>;

impl SiteStore {
    /// Load the store. `persist=false` (ephemeral profile) keeps all
    /// decisions in RAM, matching the pre-H5 behavior.
    pub fn load(persist: bool) -> Store {
        let path = persist.then(default_path);
        let data = path
            .as_deref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        Rc::new(SiteStore {
            path,
            data: RefCell::new(data),
        })
    }

    /// In-memory store for tests.
    #[cfg(test)]
    pub fn ephemeral() -> Store {
        Self::load(false)
    }

    pub fn permission(&self, host: &str, kind: &str) -> Option<bool> {
        self.data
            .borrow()
            .permissions
            .get(&format!("{host}:{kind}"))
            .copied()
    }

    pub fn set_permission(&self, host: &str, kind: &str, allow: bool) {
        self.data
            .borrow_mut()
            .permissions
            .insert(format!("{host}:{kind}"), allow);
        self.save();
    }

    /// Forget every decision for one host (or all hosts with None) —
    /// the reset story for "I mis-answered a prompt".
    pub fn clear_permissions(&self, host: Option<&str>) -> usize {
        let mut data = self.data.borrow_mut();
        let before = data.permissions.len();
        match host {
            Some(host) => {
                let prefix = format!("{host}:");
                data.permissions.retain(|k, _| !k.starts_with(&prefix));
            }
            None => data.permissions.clear(),
        }
        let removed = before - data.permissions.len();
        drop(data);
        if removed > 0 {
            self.save();
        }
        removed
    }

    pub fn zoom(&self, host: &str) -> Option<f64> {
        self.data.borrow().zoom.get(host).copied()
    }

    /// Remember a zoom level; 100% (within rounding) clears the entry
    /// so the store only holds real preferences.
    pub fn set_zoom(&self, host: &str, level: f64) {
        if host.is_empty() {
            return;
        }
        let mut data = self.data.borrow_mut();
        if (level - 1.0).abs() < 0.001 {
            if data.zoom.remove(host).is_none() {
                return; // nothing changed; skip the disk write
            }
        } else {
            data.zoom.insert(host.to_string(), level);
        }
        drop(data);
        self.save();
    }

    /// Write-through. The file is tiny (a few KB at most), so no
    /// debounce: a crash must not lose a permission decision the user
    /// just made, or the next start re-asks it.
    fn save(&self) {
        let Some(path) = &self.path else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let json = match serde_json::to_string_pretty(&*self.data.borrow()) {
            Ok(json) => json,
            Err(_) => return,
        };
        // Atomic-enough: rename over the old file so a crash mid-write
        // cannot leave truncated JSON (which load treats as empty).
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

fn default_path() -> PathBuf {
    glib::user_data_dir().join("hwatud").join("site.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ephemeral_store_remembers_in_ram_only() {
        let store = SiteStore::ephemeral();
        assert_eq!(store.permission("example.com", "camera"), None);
        store.set_permission("example.com", "camera", true);
        assert_eq!(store.permission("example.com", "camera"), Some(true));
        store.set_permission("example.com", "microphone", false);
        assert_eq!(store.permission("example.com", "microphone"), Some(false));
        // Different host is independent.
        assert_eq!(store.permission("other.com", "camera"), None);
    }

    #[test]
    fn zoom_default_level_clears_entry() {
        let store = SiteStore::ephemeral();
        store.set_zoom("example.com", 1.25);
        assert_eq!(store.zoom("example.com"), Some(1.25));
        store.set_zoom("example.com", 1.0);
        assert_eq!(store.zoom("example.com"), None);
        // Empty host never stored.
        store.set_zoom("", 2.0);
        assert_eq!(store.zoom(""), None);
    }

    #[test]
    fn clear_permissions_by_host_and_all() {
        let store = SiteStore::ephemeral();
        store.set_permission("a.com", "camera", true);
        store.set_permission("a.com", "notifications", false);
        store.set_permission("b.com", "camera", true);
        assert_eq!(store.clear_permissions(Some("a.com")), 2);
        assert_eq!(store.permission("a.com", "camera"), None);
        assert_eq!(store.permission("b.com", "camera"), Some(true));
        assert_eq!(store.clear_permissions(None), 1);
        assert_eq!(store.permission("b.com", "camera"), None);
    }

    #[test]
    fn roundtrips_through_disk() {
        let dir = std::env::temp_dir().join(format!("hwatu-sitedata-{}", std::process::id()));
        let path = dir.join("site.json");
        let store = SiteStore {
            path: Some(path.clone()),
            data: RefCell::new(Data::default()),
        };
        store.set_permission("example.com", "camera", true);
        store.set_zoom("example.com", 1.5);

        let raw = std::fs::read_to_string(&path).expect("store file written");
        let reloaded: Data = serde_json::from_str(&raw).expect("valid JSON");
        assert_eq!(reloaded.permissions.get("example.com:camera"), Some(&true));
        assert_eq!(reloaded.zoom.get("example.com"), Some(&1.5));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
