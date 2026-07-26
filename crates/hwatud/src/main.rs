// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! hwatud: the hwatu browser daemon.
//!
//! Owns the WebKit engine, a prewarmed WebView pool, and all browser
//! windows. Clients (`hana`) talk to it over a Unix socket, so opening
//! a "new browser" from the shell costs one IPC roundtrip instead of
//! an engine cold start.

mod abp;
mod adblock;
mod automation;
mod bar;
mod clock;
mod console;
mod downloads;
mod ipc_server;
mod keys;
mod launcher;
mod observe;
mod prompts;
mod search;
mod session;
mod verify;
mod window;

use gtk::prelude::*;
use gtk::{gio, glib};
use hwatu_ipc::OpenMode;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use webkit6::prelude::*;

use window::BrowserWindow;

pub const APP_ID: &str = "dev.hwatu.hwatud";

/// Shared daemon state, single-threaded (GTK main thread only).
pub struct Daemon {
    pub app: gtk::Application,
    pub windows: RefCell<HashMap<u64, Rc<BrowserWindow>>>,
    pub next_id: RefCell<u64>,
    /// One blank, fully initialized WebView kept warm so window
    /// creation never pays engine setup cost.
    pub prewarmed: RefCell<Option<webkit6::WebView>>,
    /// Built-in content blocker (on by default).
    pub adblock: adblock::Adblock,
    /// Remembered permission decisions (host+kind), daemon lifetime.
    pub prompt_memory: prompts::Memory,
    /// Resolved keybindings (defaults + ~/.config/hwatu/keys.conf).
    pub keymap: keys::Keymap,
    /// Window most recently targeted by an automation command (eval,
    /// navigate, screenshot, ...), by explicit id or resolution.
    /// Id-less commands fall back to it when several windows are open
    /// and none is focused, so an agent that opened or drove a window
    /// can keep addressing it without repeating `id`.
    pub last_target: RefCell<Option<u64>>,
    /// Idle headless windows kept for reuse by `hwatu check`, so
    /// back-to-back checks navigate a warm window instead of paying
    /// window construction + prewarm-refill per check. Entries carry
    /// a park token so a TTL timer from an earlier park of the same
    /// window cannot close a later park, plus a "was file-origin"
    /// flag: WebKit swaps web processes when a navigation leaves a
    /// `file:` document for a network one (measured ~650 ms on this
    /// path, vs ~240 ms for a fresh window), so http-target checks
    /// must not adopt a file-origin park.
    pub check_pool: RefCell<Vec<(u64, u64, bool)>>,
    /// Speculative loads awaiting adoption: url -> (window id, park
    /// token). `hwatu prefetch <url>` starts the load; the next
    /// `check` of the same URL claims the window instead of paying
    /// the navigation. Tokens work like [`Self::check_pool`]'s.
    pub prefetch_pool: RefCell<Vec<(String, u64, u64)>>,
    /// Debounce timer for crash-resilience session snapshots.
    session_save_timer: RefCell<Option<glib::SourceId>>,
}

impl Daemon {
    fn new(app: gtk::Application) -> Rc<Self> {
        Rc::new(Self {
            app,
            windows: RefCell::new(HashMap::new()),
            next_id: RefCell::new(1),
            prewarmed: RefCell::new(None),
            adblock: adblock::Adblock::default(),
            prompt_memory: prompts::Memory::default(),
            keymap: keys::Keymap::load(),
            last_target: RefCell::new(None),
            check_pool: RefCell::new(Vec::new()),
            prefetch_pool: RefCell::new(Vec::new()),
            session_save_timer: RefCell::new(None),
        })
    }

    /// Take the warm WebView (or build one) and immediately warm the next.
    pub fn take_webview(self: &Rc<Self>) -> webkit6::WebView {
        let view = self.prewarmed.borrow_mut().take().unwrap_or_else(|| {
            let view = window::build_webview();
            self.adblock.apply_to(&view);
            view
        });
        // The pool deep-warms with an about:blank load. If adoption
        // happens mid-warm, the stale load's own Started clears the
        // window's nav_pending flag and its Finished then satisfies
        // wait_load before the real navigation begins (callers' evals
        // get destroyed by the real commit). Cancel unconditionally:
        // `is_loading` is false until a provisional load engages, so a
        // conditional stop would miss the queued warm load, and
        // stopping an idle/finished view is a no-op.
        view.stop_loading();
        self.schedule_prewarm();
        view
    }

    pub fn schedule_prewarm(self: &Rc<Self>) {
        let daemon = self.clone();
        glib::idle_add_local_once(move || {
            if daemon.prewarmed.borrow().is_none() {
                let view = window::build_webview();
                daemon.adblock.apply_to(&view);
                // Deep warm: loading about:blank realizes the web
                // process and the GPU compositor path while idle, so a
                // spawned window adopts a view whose rendering pipeline
                // is already hot (matters now that hardware
                // acceleration is Always).
                view.load_uri("about:blank");
                daemon.prewarmed.replace(Some(view));
            }
        });
    }

    pub fn alloc_id(&self) -> u64 {
        let mut n = self.next_id.borrow_mut();
        let id = *n;
        *n += 1;
        id
    }

    /// Snapshot the window set for crash recovery, debounced (2 s) so
    /// bursty navigation doesn't thrash the disk. The trailing save
    /// always runs, so the file converges on the true state.
    pub fn schedule_session_save(self: &Rc<Self>) {
        if self.session_save_timer.borrow().is_some() {
            return;
        }
        let daemon = self.clone();
        let source = glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || {
            daemon.session_save_timer.replace(None);
            daemon.save_session_now();
        });
        self.session_save_timer.replace(Some(source));
    }

    pub fn save_session_now(&self) {
        let entries: Vec<session::SessionEntry> = {
            let windows = self.windows.borrow();
            let mut infos: Vec<_> = windows.values().map(|w| w.info()).collect();
            infos.sort_by_key(|w| w.id);
            infos
                .into_iter()
                // Headless windows belong to an agent's verification
                // run, not the user's session; do not resurrect them.
                .filter(|w| !w.url.is_empty() && w.mode != OpenMode::Headless)
                .map(|w| session::SessionEntry {
                    url: w.url,
                    title: w.title,
                    app_id: w.app_id,
                    mode: w.mode,
                })
                .collect()
        };
        session::save(&entries);
    }
}

/// Point the default network session's cookie jar at a SQLite file in
/// the XDG data dir, so logins survive daemon restarts. All WebViews
/// share the default session (persistence is what makes the shared
/// engine feel like one browser instead of a fleet of incognito tabs).
fn persist_cookies() {
    let Some(session) = webkit6::NetworkSession::default() else {
        eprintln!("hwatud: no default network session; cookies will not persist");
        return;
    };
    let Some(cookies) = session.cookie_manager() else {
        eprintln!("hwatud: no cookie manager; cookies will not persist");
        return;
    };
    let dir = glib::user_data_dir().join("hwatud");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!(
            "hwatud: cannot create {} ({e}); cookies will not persist",
            dir.display()
        );
        return;
    }
    let path = dir.join("cookies.sqlite");
    cookies.set_persistent_storage(
        &path.to_string_lossy(),
        webkit6::CookiePersistentStorage::Sqlite,
    );
}

fn main() -> glib::ExitCode {
    // Keep RAM predictable: cap glibc arena explosion under GTK threads.
    std::env::set_var("MALLOC_ARENA_MAX", "2");
    // Exact-DPR verification mode: `HWATU_DPR=<n>` pins window
    // devicePixelRatio to an integer n instead of whatever the session
    // compositor imposes. Root cause of the "headless dpr leak": GTK
    // derives surface scale from the *monitors* even for unmapped
    // (headless) surfaces, so a niri output at scale 1.25 makes WebKit
    // report dpr 2 in windows no compositor will ever show. Wayland
    // has no client-side scale override, and WebKit's web process
    // resolves the scale through its own display connection, so the
    // only lever that reaches everything is GDK_SCALE + the X11
    // backend, exported before gtk::init(). On a clean X server
    // (Xvfb, typical CI X) the pin is exact; on Xwayland the server
    // may impose its own base scale on top; resize()'s measure-and-
    // correct loop still lands exact CSS-px viewports there, and the
    // reply always reports the dpr the page actually sees.
    if let Some(dpr) = hwatu_dpr() {
        if std::env::var_os("GDK_BACKEND").is_none() {
            std::env::set_var("GDK_BACKEND", "x11");
        }
        std::env::set_var("GDK_SCALE", dpr.to_string());
        println!("hwatud: pinning devicePixelRatio to {dpr} (HWATU_DPR)");
    }

    let app = gtk::Application::new(Some(APP_ID), gio::ApplicationFlags::NON_UNIQUE);
    // Daemon lives even with zero windows open.
    let _hold = app.hold();

    app.connect_activate(|_| {});
    app.connect_startup(move |app| {
        bar::install_css();
        // Reclaim session blobs orphaned by a crashed/killed daemon.
        window::sweep_discard_dir();
        let daemon = Daemon::new(app.clone());
        // Cookies persist across daemon restarts. WebKit's default
        // network session keeps its jar in RAM only until told
        // otherwise, which makes every restart look like a brand-new
        // browser: logins vanish, and sites like GitHub answer the
        // fresh jar with their strictest anti-bot login path (device
        // verification, captcha). Must run before any WebView exists.
        persist_cookies();
        // Internal pages (hwatu://launcher) before any WebView exists.
        launcher::register_scheme(&daemon);
        adblock::Adblock::init(&daemon);
        daemon.schedule_prewarm();
        if let Err(e) = ipc_server::start(daemon.clone()) {
            eprintln!("hwatud: failed to start IPC server: {e}");
            std::process::exit(1);
        }
        // Crash resilience: a leftover session file means the previous
        // daemon died uncleanly (clean quits delete it); reopen its
        // windows. After the socket is up so spawn timing stays honest.
        let leftovers = session::take();
        if !leftovers.is_empty() {
            println!(
                "hwatud: restoring {} window(s) from a previous session",
                leftovers.len()
            );
            for entry in leftovers {
                // Reopen in the saved mode: a background window (agent
                // verification, WM-rule-hidden) must not come back as a
                // focused Normal window after a crash.
                BrowserWindow::open(&daemon, Some(entry.url), entry.app_id, entry.mode);
            }
        }
        println!(
            "hwatud: listening on {}",
            hwatu_ipc::socket_path().display()
        );
        // One diagnostic line: answers most "scrolling is janky on my
        // device" reports without a debug build. Smoothness scales
        // with the distro's WebKitGTK (2.46+ = Skia GPU painting).
        println!(
            "hwatud: webkitgtk {}.{}.{}, session {}, renderer {}",
            webkit6::functions::major_version(),
            webkit6::functions::minor_version(),
            webkit6::functions::micro_version(),
            std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".into()),
            std::env::var("GSK_RENDERER").unwrap_or_else(|_| "auto".into()),
        );
    });

    app.run_with_args::<&str>(&[])
}

/// `HWATU_DPR=<positive integer>`: pin devicePixelRatio for exact-DPR
/// verification matrices. Unset, empty, zero, negative, and
/// non-integer values mean "session default". Fractional pins are not
/// accepted: GDK_SCALE is integer-only, so honesty beats rounding.
fn hwatu_dpr() -> Option<i32> {
    parse_dpr(std::env::var("HWATU_DPR").ok().as_deref())
}

fn parse_dpr(raw: Option<&str>) -> Option<i32> {
    raw?.trim().parse::<i32>().ok().filter(|&n| n > 0)
}

#[cfg(test)]
mod tests {
    use super::parse_dpr;

    /// The DPR pin only accepts positive integers: GDK_SCALE cannot
    /// express fractions, and 0/negative are nonsense. Everything
    /// else must fall back to the session default rather than guess.
    #[test]
    fn dpr_env_parsing() {
        assert_eq!(parse_dpr(None), None);
        assert_eq!(parse_dpr(Some("")), None);
        assert_eq!(parse_dpr(Some("0")), None);
        assert_eq!(parse_dpr(Some("-1")), None);
        assert_eq!(parse_dpr(Some("1.5")), None);
        assert_eq!(parse_dpr(Some("abc")), None);
        assert_eq!(parse_dpr(Some("1")), Some(1));
        assert_eq!(parse_dpr(Some(" 2 ")), Some(2));
        assert_eq!(parse_dpr(Some("3")), Some(3));
    }
}
