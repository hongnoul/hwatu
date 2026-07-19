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
mod downloads;
mod ipc_server;
mod keys;
mod launcher;
mod prompts;
mod search;
mod session;
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
                })
                .collect()
        };
        session::save(&entries);
    }
}

fn main() -> glib::ExitCode {
    // Keep RAM predictable: cap glibc arena explosion under GTK threads.
    std::env::set_var("MALLOC_ARENA_MAX", "2");

    let app = gtk::Application::new(Some(APP_ID), gio::ApplicationFlags::NON_UNIQUE);
    // Daemon lives even with zero windows open.
    let _hold = app.hold();

    app.connect_activate(|_| {});
    app.connect_startup(move |app| {
        bar::install_css();
        // Reclaim session blobs orphaned by a crashed/killed daemon.
        window::sweep_discard_dir();
        let daemon = Daemon::new(app.clone());
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
                BrowserWindow::open(&daemon, Some(entry.url), entry.app_id, OpenMode::Normal);
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
