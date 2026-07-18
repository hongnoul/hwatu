//! Built-in content blocking, on by default.
//!
//! Filters are compiled by WebKit's content-extension engine
//! (`UserContentFilterStore`) into bytecode evaluated natively in the
//! network process: zero JavaScript in the hot path, zero per-page
//! cost beyond the compiled DFA. This is the same machinery Safari
//! content blockers use.
//!
//! Pipeline: ABP filter lists (embedded baseline, or downloaded
//! EasyList + EasyPrivacy after `hwatu adblock update`) -> abp.rs
//! converter -> JSON -> WebKit compile (cached on disk, keyed by a
//! source hash, so warm daemon starts skip compilation entirely).
//!
//! Toggle: `hwatu adblock on|off` (persisted), or the HWATU_ADBLOCK
//! env var (0/off to disable) which overrides the config at startup.

use crate::{abp, Daemon};
use gtk::gio;
use serde::{Deserialize, Serialize};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use webkit6::prelude::*;

/// Shipped in the binary so blocking works on first run, offline.
const BASELINE: &str = include_str!("../assets/baseline-filters.txt");

/// Identifier inside the WebKit filter store.
const FILTER_ID: &str = "hwatu-adblock";

/// Full lists fetched by `hwatu adblock update`.
const LIST_URLS: &[(&str, &str)] = &[
    ("easylist.txt", "https://easylist.to/easylist/easylist.txt"),
    (
        "easyprivacy.txt",
        "https://easylist.to/easylist/easyprivacy.txt",
    ),
];

pub struct Adblock {
    enabled: Cell<bool>,
    filter: RefCell<Option<webkit6::UserContentFilter>>,
    rules: Cell<usize>,
    source: RefCell<String>,
    compiling: Cell<bool>,
    updating: Cell<bool>,
}

impl Default for Adblock {
    fn default() -> Self {
        Self {
            enabled: Cell::new(true),
            filter: RefCell::new(None),
            rules: Cell::new(0),
            source: RefCell::new(String::new()),
            compiling: Cell::new(false),
            updating: Cell::new(false),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Config {
    #[serde(default = "default_true")]
    adblock: bool,
}

/// On by default: derive(Default) would give `false` when no config
/// file exists yet, silently shipping adblock disabled.
impl Default for Config {
    fn default() -> Self {
        Self { adblock: true }
    }
}

fn default_true() -> bool {
    true
}

/// Metadata for the compiled-ruleset cache: when the source hash
/// matches, WebKit's (slow) compile step is skipped via store.load().
#[derive(Serialize, Deserialize)]
struct CacheMeta {
    hash: u64,
    rules: usize,
    source: String,
}

fn config_path() -> PathBuf {
    glib::user_config_dir().join("hwatu").join("config.json")
}

fn filters_dir() -> PathBuf {
    glib::user_data_dir().join("hwatu").join("filters")
}

fn store_dir() -> PathBuf {
    glib::user_cache_dir().join("hwatu").join("content-filters")
}

fn meta_path() -> PathBuf {
    store_dir().join("meta.json")
}

fn load_config() -> Config {
    std::fs::read_to_string(config_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_config(cfg: &Config) {
    let path = config_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string_pretty(cfg) {
        let _ = std::fs::write(&path, json);
    }
}

/// HWATU_ADBLOCK=0/off/false disables, =1/on/true enables; unset
/// falls through to the persisted config (default: enabled).
fn initial_enabled() -> bool {
    match std::env::var("HWATU_ADBLOCK").as_deref() {
        Ok("0") | Ok("off") | Ok("false") | Ok("no") => false,
        Ok("1") | Ok("on") | Ok("true") | Ok("yes") => true,
        _ => load_config().adblock,
    }
}

fn fnv1a(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in data {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Collect filter source text: downloaded lists if present, otherwise
/// the embedded baseline; plus the user's own filters.txt always.
fn gather_source() -> (String, String) {
    let mut text = String::new();
    let mut names: Vec<String> = Vec::new();

    let dir = filters_dir();
    let mut lists: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|e| e == "txt"))
                .collect()
        })
        .unwrap_or_default();
    lists.sort();
    for path in &lists {
        if let Ok(s) = std::fs::read_to_string(path) {
            text.push_str(&s);
            text.push('\n');
            if let Some(name) = path.file_stem().and_then(|n| n.to_str()) {
                names.push(name.to_string());
            }
        }
    }
    if names.is_empty() {
        text.push_str(BASELINE);
        names.push("embedded baseline".into());
    }

    let user = glib::user_config_dir().join("hwatu").join("filters.txt");
    if let Ok(s) = std::fs::read_to_string(&user) {
        text.push_str(&s);
        names.push("user filters.txt".into());
    }

    (text, names.join(" + "))
}

impl Adblock {
    /// Read config, then compile (or cache-load) the ruleset off the
    /// main thread and apply it to every WebView.
    pub fn init(daemon: &Rc<Daemon>) {
        daemon.adblock.enabled.set(initial_enabled());
        Self::rebuild(daemon);
    }

    pub fn status(&self) -> hwatu_ipc::AdblockStatus {
        hwatu_ipc::AdblockStatus {
            enabled: self.enabled.get(),
            rules: if self.filter.borrow().is_some() {
                self.rules.get()
            } else {
                0
            },
            source: self.source.borrow().clone(),
            compiling: self.compiling.get(),
            updating: self.updating.get(),
        }
    }

    pub fn set_enabled(daemon: &Rc<Daemon>, enabled: bool) {
        daemon.adblock.enabled.set(enabled);
        let mut cfg = load_config();
        cfg.adblock = enabled;
        save_config(&cfg);
        Self::apply_all(daemon);
    }

    /// Add or remove the compiled filter on one WebView according to
    /// the current toggle. Idempotent; called on prewarmed views too.
    pub fn apply_to(&self, view: &webkit6::WebView) {
        let Some(ucm) = view.user_content_manager() else {
            return;
        };
        ucm.remove_filter_by_id(FILTER_ID);
        if self.enabled.get() {
            if let Some(filter) = &*self.filter.borrow() {
                ucm.add_filter(filter);
            }
        }
    }

    fn apply_all(daemon: &Rc<Daemon>) {
        for win in daemon.windows.borrow().values() {
            if let Some(view) = win.live_webview() {
                daemon.adblock.apply_to(&view);
            }
        }
        if let Some(view) = &*daemon.prewarmed.borrow() {
            daemon.adblock.apply_to(view);
        }
    }

    /// Convert the filter lists and hand them to WebKit. Conversion
    /// runs in a worker thread; a source-hash cache means an unchanged
    /// list resolves to a fast store.load() instead of a recompile.
    fn rebuild(daemon: &Rc<Daemon>) {
        if daemon.adblock.compiling.replace(true) {
            return; // already compiling
        }
        let daemon = daemon.clone();
        glib::spawn_future_local(async move {
            let converted = gio::spawn_blocking(|| {
                let (text, names) = gather_source();
                let c = abp::convert(text.lines());
                (c, names)
            })
            .await;
            let Ok((converted, names)) = converted else {
                daemon.adblock.compiling.set(false);
                return;
            };

            let dir = store_dir();
            let _ = std::fs::create_dir_all(&dir);
            let store =
                webkit6::UserContentFilterStore::new(&dir.to_string_lossy());

            let hash = fnv1a(converted.json.as_bytes());
            let cached = std::fs::read_to_string(meta_path())
                .ok()
                .and_then(|s| serde_json::from_str::<CacheMeta>(&s).ok())
                .is_some_and(|m| m.hash == hash);

            let result = if cached {
                store.load_future(FILTER_ID).await
            } else {
                let bytes = glib::Bytes::from_owned(converted.json.clone().into_bytes());
                store.save_future(FILTER_ID, &bytes).await
            };

            match result {
                Ok(filter) => {
                    if !cached {
                        let meta = CacheMeta {
                            hash,
                            rules: converted.rules,
                            source: names.clone(),
                        };
                        if let Ok(json) = serde_json::to_string(&meta) {
                            let _ = std::fs::write(meta_path(), json);
                        }
                    }
                    daemon.adblock.filter.replace(Some(filter));
                    daemon.adblock.rules.set(converted.rules);
                    daemon.adblock.source.replace(names);
                    daemon.adblock.compiling.set(false);
                    Self::apply_all(&daemon);
                    println!(
                        "hwatud: adblock ready ({} rules, {} lines skipped)",
                        converted.rules, converted.skipped
                    );
                }
                Err(e) => {
                    daemon.adblock.compiling.set(false);
                    // A stale cache entry can fail to load (WebKit
                    // version bump); invalidate and recompile once.
                    if cached {
                        let _ = std::fs::remove_file(meta_path());
                        Self::rebuild(&daemon);
                    } else {
                        eprintln!("hwatud: adblock compile failed: {e}");
                    }
                }
            }
        });
    }

    /// Download EasyList + EasyPrivacy, then rebuild. Runs fully
    /// async; `hwatu adblock status` reports `updating` meanwhile.
    pub fn update(daemon: &Rc<Daemon>) {
        if daemon.adblock.updating.replace(true) {
            return;
        }
        let daemon = daemon.clone();
        glib::spawn_future_local(async move {
            let fetched = gio::spawn_blocking(|| {
                let dir = filters_dir();
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                for (name, url) in LIST_URLS {
                    let out = std::process::Command::new("curl")
                        .args(["--fail", "--silent", "--location", "--max-time", "60", url])
                        .output()
                        .map_err(|e| format!("curl not available: {e}"))?;
                    if !out.status.success() {
                        return Err(format!("download failed: {url}"));
                    }
                    // Write atomically so a partial download never
                    // becomes the active list.
                    let tmp = dir.join(format!("{name}.tmp"));
                    std::fs::write(&tmp, &out.stdout).map_err(|e| e.to_string())?;
                    std::fs::rename(&tmp, dir.join(name)).map_err(|e| e.to_string())?;
                }
                Ok::<(), String>(())
            })
            .await;

            daemon.adblock.updating.set(false);
            match fetched {
                Ok(Ok(())) => Self::rebuild(&daemon),
                Ok(Err(e)) => eprintln!("hwatud: adblock update failed: {e}"),
                Err(_) => eprintln!("hwatud: adblock update task panicked"),
            }
        });
    }
}
