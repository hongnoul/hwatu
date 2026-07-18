//! hwatud: the hwatu browser daemon.
//!
//! Owns the WebKit engine, a prewarmed WebView pool, and all browser
//! windows. Clients (`hana`) talk to it over a Unix socket, so opening
//! a "new browser" from the shell costs one IPC roundtrip instead of
//! an engine cold start.

mod ipc_server;
mod window;

use gtk::prelude::*;
use gtk::{gio, glib};
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
}

impl Daemon {
    fn new(app: gtk::Application) -> Rc<Self> {
        Rc::new(Self {
            app,
            windows: RefCell::new(HashMap::new()),
            next_id: RefCell::new(1),
            prewarmed: RefCell::new(None),
        })
    }

    /// Take the warm WebView (or build one) and immediately warm the next.
    pub fn take_webview(self: &Rc<Self>) -> webkit6::WebView {
        let view = self
            .prewarmed
            .borrow_mut()
            .take()
            .unwrap_or_else(window::build_webview);
        self.schedule_prewarm();
        view
    }

    pub fn schedule_prewarm(self: &Rc<Self>) {
        let daemon = self.clone();
        glib::idle_add_local_once(move || {
            if daemon.prewarmed.borrow().is_none() {
                let view = window::build_webview();
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
}

fn main() -> glib::ExitCode {
    // Keep RAM predictable: cap glibc arena explosion under GTK threads.
    std::env::set_var("MALLOC_ARENA_MAX", "2");

    let app = gtk::Application::new(Some(APP_ID), gio::ApplicationFlags::NON_UNIQUE);
    // Daemon lives even with zero windows open.
    let _hold = app.hold();

    app.connect_activate(|_| {});
    app.connect_startup(move |app| {
        let daemon = Daemon::new(app.clone());
        daemon.schedule_prewarm();
        if let Err(e) = ipc_server::start(daemon) {
            eprintln!("hwatud: failed to start IPC server: {e}");
            std::process::exit(1);
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
