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
mod blurshield;
mod clock;
mod compositor;
mod console;
mod downloads;
mod events;
mod external;
mod focusshield;
mod ipc_server;
mod keys;
mod launcher;
mod mediashim;
mod net;
mod observe;
mod opfs;
mod palette;
mod prompts;
mod search;
mod session;
mod siteua;
mod smoothwheel;
mod snapdiff;
mod trusted_input;
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

/// Signal numbers (Linux). Matching compositor.rs, no libc dependency.
mod libc_signals {
    pub const SIGHUP: i32 = 1;
    pub const SIGINT: i32 = 2;
    pub const SIGTERM: i32 = 15;
}

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
    /// Recently closed user windows, newest last, capped small.
    /// `ctrl+shift+t` (Action::ReopenClosed) pops from here. Headless
    /// windows (agent verification runs) and blank/launcher pages are
    /// never recorded.
    pub recently_closed: RefCell<Vec<session::SessionEntry>>,
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
    /// Push-IPC subscribers (`subscribe` on a held-open connection).
    pub events: events::Broker,
    /// Next hanafuda card to deal on a launcher window (see
    /// [`launcher::deal_uri`]). Wraps modulo the deck size.
    pub next_deal: RefCell<usize>,
    /// Operator security policy selected at daemon startup.
    pub security: SecurityConfig,
    /// Isolated network session used by every WebView when the daemon is
    /// running in ephemeral-profile mode. Holding the object here keeps the
    /// in-memory cookie jar alive while the daemon runs.
    pub network_session: Option<webkit6::NetworkSession>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecurityConfig {
    pub eval_enabled: bool,
    pub ephemeral_profile: bool,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            eval_enabled: true,
            ephemeral_profile: false,
        }
    }
}

impl Daemon {
    fn new(app: gtk::Application, security: SecurityConfig) -> Rc<Self> {
        let network_session = if security.ephemeral_profile {
            let session = webkit6::NetworkSession::new_ephemeral();
            session.set_persistent_credential_storage_enabled(false);
            println!("hwatud: ephemeral profile enabled (memory-only WebKit session)");
            Some(session)
        } else {
            None
        };

        Rc::new(Self {
            app,
            windows: RefCell::new(HashMap::new()),
            next_id: RefCell::new(1),
            prewarmed: RefCell::new(None),
            adblock: adblock::Adblock::default(),
            prompt_memory: prompts::Memory::default(),
            keymap: keys::Keymap::load(),
            last_target: RefCell::new(None),
            recently_closed: RefCell::new(Vec::new()),
            check_pool: RefCell::new(Vec::new()),
            prefetch_pool: RefCell::new(Vec::new()),
            session_save_timer: RefCell::new(None),
            events: events::Broker::default(),
            next_deal: RefCell::new(0),
            security,
            network_session,
        })
    }

    /// Deal the next hanafuda card index and advance the counter,
    /// wrapping after the whole deck.
    pub fn take_deal(&self) -> usize {
        let mut n = self.next_deal.borrow_mut();
        let deal = *n;
        *n = (deal + 1) % launcher::DECK_SIZE;
        deal
    }

    /// Take the warm WebView (or build one) and immediately warm the next.
    pub fn take_webview(self: &Rc<Self>) -> webkit6::WebView {
        let view = self.prewarmed.borrow_mut().take().unwrap_or_else(|| {
            let view = window::build_webview(self);
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
                let view = window::build_webview(&daemon);
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
        if self.security.ephemeral_profile {
            return;
        }
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
        if self.security.ephemeral_profile {
            return;
        }
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
fn persist_cookies(security: SecurityConfig) {
    if security.ephemeral_profile {
        println!("hwatud: ephemeral profile enabled; persistent cookies disabled");
        return;
    }
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

/// Whitelist media directories in the web-process sandbox.
///
/// Page resources are proxied by the network process, but GStreamer
/// media decode runs *inside* the sandboxed web process and opens
/// `file://` URLs directly with GstFileSrc. The bwrap sandbox denies
/// $HOME by default, so playing a local video (a downloaded file, a
/// recording) fails with MEDIA_ERR_SRC_NOT_SUPPORTED even though the
/// media document itself loads fine — the demuxer never gets to read
/// a byte. Mount the XDG user content dirs read-only, plus any extra
/// colon-separated paths from HWATU_SANDBOX_PATHS.
///
/// Must run before the first web process spawns (WebKit aborts on
/// late additions), hence called from startup before the prewarm.
fn open_sandbox_for_media() {
    let Some(context) = webkit6::WebContext::default() else {
        return;
    };
    let mut paths: Vec<std::path::PathBuf> = [
        glib::UserDirectory::Downloads,
        glib::UserDirectory::Videos,
        glib::UserDirectory::Music,
        glib::UserDirectory::Pictures,
        glib::UserDirectory::Desktop,
        glib::UserDirectory::Documents,
    ]
    .into_iter()
    .filter_map(glib::user_special_dir)
    .collect();
    if let Ok(extra) = std::env::var("HWATU_SANDBOX_PATHS") {
        paths.extend(
            extra
                .split(':')
                .filter(|s| !s.is_empty())
                .map(std::path::PathBuf::from),
        );
    }
    let home = glib::home_dir();
    let runtime = glib::user_runtime_dir();
    paths.sort();
    paths.dedup();
    for path in paths {
        // WebKit hard-rejects $HOME itself (2.40+) and nonexistent
        // paths; both happen with unconfigured xdg-user-dirs, where
        // special dirs fall back to $HOME. A read-only mount over an
        // ancestor of XDG_RUNTIME_DIR shadows WebKit's own IPC bus
        // mount and kills every web process at spawn, so refuse it.
        if path == home || !path.is_dir() || runtime.starts_with(&path) {
            continue;
        }
        context.add_path_to_sandbox(&path, true);
    }
}

fn main() -> glib::ExitCode {
    let security = match parse_security_args(std::env::args().skip(1)) {
        Ok(ParseSecurity::Run(security)) => security,
        Ok(ParseSecurity::Help) => {
            println!("usage: hwatud [--no-eval] [--ephemeral-profile]");
            return glib::ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("hwatud: {message}");
            eprintln!("usage: hwatud [--no-eval] [--ephemeral-profile]");
            return glib::ExitCode::FAILURE;
        }
    };
    // Keep RAM predictable: cap glibc arena explosion under GTK threads.
    std::env::set_var("MALLOC_ARENA_MAX", "2");
    // Full-refresh-rate scrolling: WebKitGTK's default DMA-BUF
    // presentation path paces `frameDone` at 60Hz on setups where its
    // vblank monitor falls back to a fixed 60fps timer (observed on
    // 144Hz Wayland/niri, WebKitGTK 2.52; idle rAF hits 144 but any
    // scroll/repaint work drops to ~59). Disabling the DMA-BUF
    // renderer falls back to the legacy EGLImage path, which follows
    // the GTK frame clock: measured idle=142.9/scroll=142.9 vs
    // 62.5/58.8 on the same fixture, with equal CPU cost and vsync
    // intact (docs/research-input-lag.md "144Hz scroll cap"). WebKit
    // treats any value other than "0" as "disable", so exporting
    // WEBKIT_DISABLE_DMABUF_RENDERER=0 restores the DMA-BUF path if
    // this fallback ever misbehaves; explicit user env always wins.
    // Must run before GTK/WebKit init; web processes inherit the env.
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
    // Display-free operation (roadmap G4): with no usable
    // WAYLAND_DISPLAY/DISPLAY, spawn a managed headless child
    // compositor and point GTK at it. Must run before any GTK/GDK
    // call (GTK resolves its display connection at init). The guard
    // supervises the child; dropping it (daemon exit) kills the
    // compositor, and PDEATHSIG covers unclean exits.
    let _compositor = match compositor::ensure_display() {
        Ok(guard) => guard,
        Err(e) => {
            eprintln!("hwatud: {e}");
            return glib::ExitCode::FAILURE;
        }
    };
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
        if !security.ephemeral_profile {
            window::sweep_discard_dir();
        }
        let daemon = Daemon::new(app.clone(), security);
        // Cookies persist across daemon restarts. WebKit's default
        // network session keeps its jar in RAM only until told
        // otherwise, which makes every restart look like a brand-new
        // browser: logins vanish, and sites like GitHub answer the
        // fresh jar with their strictest anti-bot login path (device
        // verification, captcha). Must run before any WebView exists.
        persist_cookies(security);
        // Local media playback needs the web-process sandbox to see
        // the files. Before any web process exists (hard requirement).
        open_sandbox_for_media();
        // Internal pages (hwatu://launcher) before any WebView exists.
        launcher::register_scheme();
        adblock::Adblock::init(&daemon);
        daemon.schedule_prewarm();
        if let Err(e) = ipc_server::start(daemon.clone()) {
            eprintln!("hwatud: failed to start IPC server: {e}");
            std::process::exit(1);
        }
        // Crash resilience: a leftover session file means the previous
        // daemon died uncleanly (clean quits delete it); reopen its
        // windows. After the socket is up so spawn timing stays honest.
        let leftovers = if security.ephemeral_profile {
            Vec::new()
        } else {
            session::take()
        };
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
        // SIGTERM/SIGINT/SIGHUP (logout, `kill` during an update,
        // Ctrl+C in a terminal): the default action kills the process
        // between debounced saves, losing any window opened or
        // navigated in the last 2 s. Flush the snapshot synchronously
        // and exit; the file is left behind on purpose so the next
        // start restores the windows (only `hwatu quit` clears it).
        for signum in [
            libc_signals::SIGTERM,
            libc_signals::SIGINT,
            libc_signals::SIGHUP,
        ] {
            let daemon = daemon.clone();
            glib::unix_signal_add_local(signum, move || {
                daemon.save_session_now();
                daemon.app.quit();
                glib::ControlFlow::Break
            });
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParseSecurity {
    Run(SecurityConfig),
    Help,
}

fn parse_security_args<I, S>(args: I) -> Result<ParseSecurity, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut config = SecurityConfig::default();

    if std::env::var_os("HWATUD_NO_EVAL").is_some_and(|v| !v.is_empty() && v != "0") {
        config.eval_enabled = false;
    }
    if std::env::var_os("HWATUD_EPHEMERAL_PROFILE").is_some_and(|v| !v.is_empty() && v != "0") {
        config.ephemeral_profile = true;
    }

    for arg in args {
        match arg.as_ref() {
            "--no-eval" => config.eval_enabled = false,
            "--ephemeral-profile" | "--ephemeral" => config.ephemeral_profile = true,
            "--help" | "-h" => return Ok(ParseSecurity::Help),
            other => return Err(format!("unknown argument `{other}`")),
        }
    }
    Ok(ParseSecurity::Run(config))
}

#[cfg(test)]
mod tests {
    use super::{parse_dpr, parse_security_args, ParseSecurity, SecurityConfig};

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

    #[test]
    fn security_args_default_open_profile() {
        assert_eq!(
            parse_security_args(std::iter::empty::<&str>()).unwrap(),
            ParseSecurity::Run(SecurityConfig::default())
        );
    }

    #[test]
    fn security_args_accept_eval_and_profile_opt_outs() {
        assert_eq!(
            parse_security_args(["--no-eval", "--ephemeral-profile"]).unwrap(),
            ParseSecurity::Run(SecurityConfig {
                eval_enabled: false,
                ephemeral_profile: true,
            })
        );
    }

    #[test]
    fn security_args_help_is_successful_control_flow() {
        assert_eq!(
            parse_security_args(["--help"]).unwrap(),
            ParseSecurity::Help
        );
    }

    #[test]
    fn security_args_reject_unknown_flags() {
        assert!(parse_security_args(["--cookies-please"]).is_err());
    }
}
