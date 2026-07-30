// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! Display-free operation (roadmap G4).
//!
//! GTK (and therefore WebKitGTK) cannot initialize without a display
//! connection, but the daemon's core workload — check/render/eval/
//! shot — never needs a *visible* display. When hwatud starts with no
//! usable `WAYLAND_DISPLAY` or `DISPLAY` (CI, headless boxes, ssh), it
//! enters display-free mode: spawn a child Wayland compositor on a
//! wlroots headless backend, point `WAYLAND_DISPLAY` at its socket,
//! and init GTK against that. With a display present, this module is
//! a no-op and daemon behavior is byte-identical to before.
//!
//! Design choices, documented:
//!
//! - **Managed child compositor over a second engine.** The roadmap
//!   evaluated WPE WebKit as a headless-only backend; that means a
//!   second engine to maintain and behavioral drift between headed and
//!   headless runs. A child compositor keeps one engine and one code
//!   path: the only difference in display-free mode is who owns the
//!   Wayland socket.
//! - **Probe order: cage, labwc, sway.** All three package wlroots'
//!   headless backend (`WLR_BACKENDS=headless`). cage is the smallest
//!   (a kiosk compositor, purpose-built to host one app); labwc needs
//!   no configuration; sway is the most widely packaged but wants a
//!   config file (we hand it an empty one). Whichever exists first
//!   wins; if none is installed the daemon exits with an error naming
//!   all three.
//! - **Private XDG_RUNTIME_DIR for the child.** wlroots picks its own
//!   socket name (`wayland-N`, first free slot), so instead of racing
//!   to guess N in a shared dir, the child gets a private runtime dir
//!   and we scan it for the socket. `WAYLAND_DISPLAY` is then set to
//!   the socket's *absolute path* (supported by libwayland >= 1.15),
//!   which both GTK and WebKit's web processes resolve.
//! - **No orphans.** The compositor runs under a `/bin/sh` supervisor
//!   that gets `PR_SET_PDEATHSIG(SIGTERM)` and forwards it. The
//!   indirection is load-bearing: Linux clears the parent-death signal
//!   on exec of a binary with file capabilities, and distro sway ships
//!   with `cap_sys_nice=ep`, so PDEATHSIG set directly on the
//!   compositor silently evaporates. `sh` has no file caps, keeps the
//!   signal through every daemon death (including SIGKILL and the
//!   `hwatu quit` path's `process::exit`, which skips `Drop`), and its
//!   TERM trap kills the compositor. The supervisor also runs in its
//!   own process group so shutdown can signal the whole tree, and
//!   [`Compositor`]'s `Drop` covers the normal-return path and removes
//!   the private runtime dir.
//! - **Compositor death is fatal, loudly.** If the child exits while
//!   the daemon runs, GTK's display connection is gone and GTK cannot
//!   re-attach to a new one; any recovery would mean restarting the
//!   whole process anyway. A monitor thread reaps the child and, on
//!   unexpected exit, prints a clear one-line error and exits(1) —
//!   better than the "Broken pipe" abort GTK would otherwise produce.

use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// True once the daemon has entered display-free mode. Read by IPC
/// handlers that need a real session display (e.g. `focus`).
static DISPLAY_FREE: AtomicBool = AtomicBool::new(false);

pub fn display_free() -> bool {
    DISPLAY_FREE.load(Ordering::Relaxed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayMode {
    /// A session display is reachable; change nothing.
    Session,
    /// No usable display; a managed child compositor is required.
    DisplayFree,
}

/// Detect the display mode from the environment.
///
/// `WAYLAND_DISPLAY` counts only if its socket actually accepts a
/// connection (a stale name inherited into a systemd unit or CI env
/// must not fool us into a GTK init that will fail). `DISPLAY` is
/// trusted if non-empty: verifying an X server needs Xlib, and a wrong
/// `DISPLAY` fails with today's error message, which is the status quo
/// this feature must not change.
pub fn detect_mode() -> DisplayMode {
    detect_mode_with(
        std::env::var("WAYLAND_DISPLAY").ok(),
        std::env::var("DISPLAY").ok(),
        wayland_socket_usable,
    )
}

fn detect_mode_with(
    wayland: Option<String>,
    display: Option<String>,
    wayland_usable: impl Fn(&str) -> bool,
) -> DisplayMode {
    if let Some(w) = wayland {
        if !w.trim().is_empty() && wayland_usable(&w) {
            return DisplayMode::Session;
        }
    }
    if let Some(d) = display {
        if !d.trim().is_empty() {
            return DisplayMode::Session;
        }
    }
    DisplayMode::DisplayFree
}

/// Can the named Wayland socket be connected to? Resolves relative
/// names against `XDG_RUNTIME_DIR` exactly like libwayland does.
fn wayland_socket_usable(name: &str) -> bool {
    let path = if name.starts_with('/') {
        PathBuf::from(name)
    } else {
        match std::env::var_os("XDG_RUNTIME_DIR") {
            Some(dir) => PathBuf::from(dir).join(name),
            None => return false,
        }
    };
    UnixStream::connect(path).is_ok()
}

/// Entry point called from `main()` before GTK init.
///
/// Session mode returns `Ok(None)` without touching the environment.
/// Display-free mode spawns the child compositor, exports
/// `WAYLAND_DISPLAY` (absolute socket path) and `GDK_BACKEND=wayland`,
/// and returns the supervision guard, which must be kept alive for the
/// daemon's lifetime.
pub fn ensure_display() -> Result<Option<Compositor>, String> {
    match detect_mode() {
        DisplayMode::Session => Ok(None),
        DisplayMode::DisplayFree => {
            export_software_rendering_fallback();
            let comp = Compositor::spawn()?;
            std::env::set_var("WAYLAND_DISPLAY", &comp.socket);
            std::env::set_var("GDK_BACKEND", "wayland");
            DISPLAY_FREE.store(true, Ordering::Relaxed);
            println!(
                "hwatud: display-free mode: compositor {} (pid {}) on {}",
                comp.name,
                comp.pid,
                comp.socket.display()
            );
            Ok(Some(comp))
        }
    }
}

/// GPU-less boxes (CI runners, minimal VMs) are the main audience of
/// display-free mode, and WebKitGTK's default DMA-BUF renderer aborts
/// there (`g_error` -> SIGTRAP) because buffer sharing needs a DRM
/// render node. When no usable node exists, fall back to software
/// rendering before GTK/WebKit initialize (web processes inherit the
/// env). Explicit user env always wins; boxes WITH a GPU are
/// untouched.
fn export_software_rendering_fallback() {
    if has_drm_render_node() {
        return;
    }
    for (key, value) in [
        // WebKit: render in shared memory instead of DMA-BUF.
        ("WEBKIT_DISABLE_DMABUF_RENDERER", "1"),
        // Mesa: llvmpipe instead of probing for hardware.
        ("LIBGL_ALWAYS_SOFTWARE", "1"),
        // wlroots: pure-CPU pixman compositing. GLES-on-llvmpipe is
        // the alternative and has aborted cage on GPU-less CI; pixman
        // (wlroots >= 0.15, everything packaged today) is the honest
        // renderer for a display nothing is shown on. Inherited by
        // the child compositor via spawn_candidate.
        ("WLR_RENDERER", "pixman"),
    ] {
        if std::env::var_os(key).is_none() {
            std::env::set_var(key, value);
        }
    }
    println!("hwatud: no DRM render node; falling back to software rendering");
}

/// Is there an openable DRM render node? Openable matters: the node
/// can exist while the daemon's user lacks the `render` group.
fn has_drm_render_node() -> bool {
    let Ok(entries) = std::fs::read_dir("/dev/dri") else {
        return false;
    };
    entries.flatten().any(|e| {
        e.file_name().to_string_lossy().starts_with("renderD")
            && std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(e.path())
                .is_ok()
    })
}

/// A supervised child compositor. Dropping it terminates the child
/// and removes its private runtime dir.
pub struct Compositor {
    name: &'static str,
    pid: i32,
    socket: PathBuf,
    runtime_dir: PathBuf,
    /// Set before we kill the child ourselves, so the monitor thread
    /// can tell a shutdown from a crash.
    expected_exit: Arc<AtomicBool>,
    /// Held only so stdio handles live as long as the child; the
    /// monitor thread owns reaping (via waitpid), never `Child::wait`.
    _child: Child,
}

const CANDIDATES: [&str; 3] = ["cage", "labwc", "sway"];
const SOCKET_WAIT: Duration = Duration::from_secs(8);

impl Compositor {
    fn spawn() -> Result<Self, String> {
        let runtime_dir = private_runtime_dir()
            .map_err(|e| format!("display-free mode: cannot create compositor runtime dir: {e}"))?;
        let mut probed = Vec::new();
        for name in CANDIDATES {
            let Some(bin) = find_in_path(name) else {
                probed.push(format!("{name} (not installed)"));
                continue;
            };
            let mut child = match spawn_candidate(name, &bin, &runtime_dir) {
                Ok(c) => c,
                Err(e) => {
                    probed.push(format!("{name} (spawn failed: {e})"));
                    continue;
                }
            };
            match wait_for_socket(&runtime_dir, &mut child, SOCKET_WAIT) {
                Ok(socket) => {
                    let pid = child.id() as i32;
                    let expected_exit = Arc::new(AtomicBool::new(false));
                    monitor(name, pid, expected_exit.clone());
                    return Ok(Self {
                        name,
                        pid,
                        socket,
                        runtime_dir,
                        expected_exit,
                        _child: child,
                    });
                }
                Err(why) => {
                    kill_group(child.id() as i32, SIGTERM);
                    std::thread::sleep(Duration::from_millis(100));
                    kill_group(child.id() as i32, SIGKILL);
                    let _ = child.wait();
                    probed.push(format!("{name} ({why})"));
                }
            }
        }
        let _ = std::fs::remove_dir_all(&runtime_dir);
        Err(format!(
            "no display: WAYLAND_DISPLAY and DISPLAY are both unset or unusable, and no \
             headless Wayland compositor could be started. Install one of: cage, sway, \
             labwc (any wlroots compositor with WLR_BACKENDS=headless works). \
             Probed: {}",
            probed.join(", ")
        ))
    }
}

impl Drop for Compositor {
    fn drop(&mut self) {
        self.expected_exit.store(true, Ordering::Relaxed);
        kill_group(self.pid, SIGTERM);
        // Wait briefly for the monitor thread to reap the supervisor,
        // so the runtime-dir removal below doesn't race a live child.
        // kill(pid, 0) failing (ESRCH) means reaped and gone.
        let mut gone = false;
        for _ in 0..40 {
            if unsafe { libc_kill(self.pid, 0) } != 0 {
                gone = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        if !gone {
            kill_group(self.pid, SIGKILL);
        }
        let _ = std::fs::remove_dir_all(&self.runtime_dir);
    }
}

/// Signal the supervisor's whole process group (it `setpgid`s at
/// spawn, so its pgid is its pid), falling back to the pid alone if
/// the group is already gone.
fn kill_group(pid: i32, sig: i32) {
    if unsafe { libc_kill(-pid, sig) } != 0 {
        unsafe { libc_kill(pid, sig) };
    }
}

/// Reap the child and fail fast if it dies behind our back (see the
/// module docs for why death is fatal rather than recovered).
fn monitor(name: &'static str, pid: i32, expected_exit: Arc<AtomicBool>) {
    let _ = std::thread::Builder::new()
        .name("compositor-watch".into())
        .spawn(move || {
            let mut status: i32 = 0;
            let reaped = unsafe { libc_waitpid(pid, &mut status, 0) };
            if reaped == pid && !expected_exit.load(Ordering::Relaxed) {
                eprintln!(
                    "hwatud: display-free compositor {name} (pid {pid}) exited unexpectedly; \
                     the Wayland display is gone and GTK cannot re-attach. Exiting; restart \
                     hwatud to recover."
                );
                std::process::exit(1);
            }
        });
}

/// The `/bin/sh` supervisor body. Spawns the compositor, forwards
/// SIGTERM/SIGINT/SIGHUP to it, and exits with its status. See the
/// module docs: PDEATHSIG must land on a binary without file
/// capabilities, and this is that binary.
const SUPERVISE_SH: &str = r#"
child=
trap '[ -n "$child" ] && kill -TERM "$child" 2>/dev/null' TERM INT HUP
"$@" &
child=$!
wait "$child"
st=$?
# A trapped signal interrupts the first wait; collect the child for
# real before exiting so it cannot outlive the supervisor.
wait "$child" 2>/dev/null
exit "$st"
"#;

fn spawn_candidate(name: &str, bin: &Path, runtime_dir: &Path) -> std::io::Result<Child> {
    let mut cmd = Command::new("/bin/sh");
    cmd.arg("-c").arg(SUPERVISE_SH).arg("hwatud-comp").arg(bin);
    match name {
        // cage exits when its client does; host one that never does.
        "cage" => {
            cmd.args(["--", "sleep", "infinity"]);
        }
        // sway insists on a config file; an empty one is valid and
        // keeps the user's real config out of the managed instance.
        "sway" => {
            let conf = runtime_dir.join("sway-headless.conf");
            std::fs::write(&conf, "# hwatud-managed headless compositor\n")?;
            cmd.arg("--config").arg(&conf);
        }
        // labwc starts bare.
        _ => {}
    }
    cmd.env_remove("WAYLAND_DISPLAY")
        .env_remove("DISPLAY")
        .env("XDG_RUNTIME_DIR", runtime_dir)
        .env("WLR_BACKENDS", "headless")
        .env("WLR_LIBINPUT_NO_DEVICES", "1")
        // CI runners have no GPU; pixman/llvmpipe is fine for a
        // display nothing is ever shown on.
        .env("WLR_RENDERER_ALLOW_SOFTWARE", "1")
        .stdin(Stdio::null());
    // Child output is noise in normal operation, but the only clue
    // when startup fails; `HWATUD_COMPOSITOR_LOG=<path>` captures it.
    match std::env::var_os("HWATUD_COMPOSITOR_LOG") {
        Some(path) => {
            let log = std::fs::File::create(path)?;
            cmd.stdout(log.try_clone()?).stderr(log);
        }
        None => {
            cmd.stdout(Stdio::null()).stderr(Stdio::null());
        }
    }
    // Kernel-enforced no-orphans: if the daemon dies (even SIGKILL,
    // even `process::exit` on the quit path), the supervisor gets
    // SIGTERM and forwards it. Also make the supervisor a process-
    // group leader so shutdown can signal the whole tree at once.
    // Set in the child right after fork.
    unsafe {
        cmd.pre_exec(|| {
            libc_setpgid(0, 0);
            libc_prctl(PR_SET_PDEATHSIG, SIGTERM as u64, 0, 0, 0);
            Ok(())
        });
    }
    cmd.spawn()
}

/// Poll the private runtime dir until a connectable `wayland-*` socket
/// appears, the child exits, or the deadline passes.
fn wait_for_socket(
    runtime_dir: &Path,
    child: &mut Child,
    timeout: Duration,
) -> Result<PathBuf, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(socket) = find_wayland_socket(runtime_dir) {
            if UnixStream::connect(&socket).is_ok() {
                return Ok(socket);
            }
        }
        match child.try_wait() {
            Ok(Some(status)) => return Err(format!("exited during startup: {status}")),
            Ok(None) => {}
            Err(e) => return Err(format!("wait failed: {e}")),
        }
        if Instant::now() > deadline {
            return Err(format!("no socket within {}s", timeout.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn find_wayland_socket(dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("wayland-") && !name.ends_with(".lock") {
            return Some(entry.path());
        }
    }
    None
}

/// A private, 0700 runtime dir for the child compositor, so its
/// auto-picked `wayland-N` socket name cannot collide with (or be
/// mistaken for) anything in the real session's runtime dir.
///
/// The path must stay SHORT: Unix socket paths cap at ~108 bytes
/// (`sun_path`), and sway dies at startup when its IPC socket path
/// (`<dir>/sway-ipc.<uid>.<pid>.sock`) does not fit. A deep
/// `XDG_RUNTIME_DIR` (test sandboxes, containers) would push it over,
/// so long bases fall back to the system temp dir.
fn private_runtime_dir() -> std::io::Result<PathBuf> {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.as_os_str().len() <= 60)
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join(format!("hwatud-comp-{}", std::process::id()));
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
    Ok(dir)
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

// Tiny FFI shims (matching the crate's existing style, e.g.
// session.rs's geteuid) so the daemon stays libc-crate-free.
const PR_SET_PDEATHSIG: i32 = 1;
const SIGTERM: i32 = 15;
const SIGKILL: i32 = 9;

extern "C" {
    #[link_name = "kill"]
    fn libc_kill(pid: i32, sig: i32) -> i32;
    #[link_name = "setpgid"]
    fn libc_setpgid(pid: i32, pgid: i32) -> i32;
    #[link_name = "waitpid"]
    fn libc_waitpid(pid: i32, status: *mut i32, options: i32) -> i32;
    // Real prctl is variadic; the fixed 5-arg declaration is ABI-
    // compatible on every Linux ABI Rust supports (same as libc's).
    #[link_name = "prctl"]
    fn libc_prctl(option: i32, arg2: u64, arg3: u64, arg4: u64, arg5: u64) -> i32;
}

#[cfg(test)]
mod tests {
    use super::{detect_mode_with, DisplayMode};

    fn detect(wayland: Option<&str>, display: Option<&str>, usable: bool) -> DisplayMode {
        detect_mode_with(wayland.map(String::from), display.map(String::from), |_| {
            usable
        })
    }

    /// The mode gate: display-free only when neither env var offers a
    /// usable display. A set-but-dead WAYLAND_DISPLAY must not count
    /// (stale CI env), while any non-empty DISPLAY does (X liveness is
    /// not probed; a wrong DISPLAY keeps today's failure behavior).
    #[test]
    fn mode_detection() {
        // Nothing set: display-free.
        assert_eq!(detect(None, None, false), DisplayMode::DisplayFree);
        // Empty strings are as good as unset.
        assert_eq!(detect(Some(""), Some(""), true), DisplayMode::DisplayFree);
        assert_eq!(detect(Some("  "), None, true), DisplayMode::DisplayFree);
        // Usable Wayland socket: session.
        assert_eq!(detect(Some("wayland-1"), None, true), DisplayMode::Session);
        // Set but unusable Wayland, nothing else: display-free.
        assert_eq!(
            detect(Some("wayland-1"), None, false),
            DisplayMode::DisplayFree
        );
        // DISPLAY alone is trusted.
        assert_eq!(detect(None, Some(":0"), false), DisplayMode::Session);
        // Dead Wayland + live-looking X: session (X wins).
        assert_eq!(
            detect(Some("wayland-1"), Some(":0"), false),
            DisplayMode::Session
        );
    }
}
