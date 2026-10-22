// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Justin Hong
//! OS-specific process and file primitives for the CLI. Portable modules call
//! these functions and contain no `cfg(unix)` / `cfg(windows)` branches.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::Duration;

/// Start `cmd` as the leader of a new process group (Unix) or a new process
/// group with no console window (Windows), so the whole tree can be stopped.
pub fn spawn_group_leader(cmd: &mut Command) -> &mut Command {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0)
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW)
    }
}

/// Stop a child started with [`spawn_group_leader`] and every process it
/// created. Graceful first, then forced after about 500 ms.
pub fn terminate_tree(child: &mut Child) {
    #[cfg(unix)]
    {
        let group = child.id();
        signal_group(group, SIGTERM);
        for _ in 0..20 {
            // Reap the leader before probing the group. An exited but unreaped
            // leader is still visible to kill(2).
            let leader_exited = child.try_wait().ok().flatten().is_some();
            if !signal_group(group, 0) {
                if !leader_exited {
                    let _ = child.wait();
                }
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        signal_group(group, SIGKILL);
        let _ = child.kill();
        let _ = child.wait();
    }
    #[cfg(windows)]
    {
        // taskkill /T walks the process tree. /F is needed for console
        // processes that ignore WM_CLOSE.
        let pid = child.id().to_string();
        let _ = Command::new("taskkill").args(["/PID", &pid, "/T"]).output();
        for _ in 0..20 {
            if child.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let _ = Command::new("taskkill")
            .args(["/PID", &pid, "/T", "/F"])
            .output();
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[cfg(unix)]
const SIGKILL: i32 = 9;
#[cfg(unix)]
const SIGINT: i32 = 2;
#[cfg(unix)]
const SIGTERM: i32 = 15;
#[cfg(unix)]
const SIG_ERR: usize = usize::MAX;

#[cfg(unix)]
extern "C" {
    fn kill(pid: i32, signal: i32) -> i32;
    fn signal(signal: i32, handler: usize) -> usize;
    fn siginterrupt(signal: i32, flag: i32) -> i32;
}

#[cfg(unix)]
fn signal_group(group: u32, signal: i32) -> bool {
    let Ok(group) = i32::try_from(group) else {
        return false;
    };
    // Negative PIDs address the whole POSIX process group.
    unsafe { kill(-group, signal) == 0 }
}

/// True while any process in the group led by `group` is alive.
#[cfg(all(unix, test))]
pub fn group_alive(group: u32) -> bool {
    signal_group(group, 0)
}

/// Process-wide interrupt handling: while the guard is alive, Ctrl-C and
/// termination requests call `on_cancel` instead of killing the process.
pub struct InterruptGuard {
    #[cfg(unix)]
    previous_int: usize,
    #[cfg(unix)]
    previous_term: usize,
}

#[cfg(unix)]
static ON_CANCEL: std::sync::OnceLock<fn()> = std::sync::OnceLock::new();

#[cfg(unix)]
extern "C" fn unix_handler(_signal: i32) {
    if let Some(f) = ON_CANCEL.get() {
        f();
    }
}

#[cfg(windows)]
static ON_CANCEL_WIN: std::sync::OnceLock<fn()> = std::sync::OnceLock::new();

#[cfg(windows)]
extern "system" fn windows_handler(_ctrl: u32) -> i32 {
    if let Some(f) = ON_CANCEL_WIN.get() {
        f();
    }
    1
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn SetConsoleCtrlHandler(handler: Option<extern "system" fn(u32) -> i32>, add: i32) -> i32;
}

impl InterruptGuard {
    pub fn install(on_cancel: fn()) -> Result<Self, String> {
        #[cfg(unix)]
        {
            let _ = ON_CANCEL.set(on_cancel);
            let handler = unix_handler as *const () as usize;
            let previous_int = unsafe { signal(SIGINT, handler) };
            if previous_int == SIG_ERR {
                return Err("install SIGINT cleanup handler failed".into());
            }
            let previous_term = unsafe { signal(SIGTERM, handler) };
            if previous_term == SIG_ERR {
                unsafe {
                    signal(SIGINT, previous_int);
                }
                return Err("install SIGTERM cleanup handler failed".into());
            }
            // Interrupt a blocking daemon read so unwinding promptly drops
            // guards that own child processes.
            unsafe {
                siginterrupt(SIGINT, 1);
                siginterrupt(SIGTERM, 1);
            }
            Ok(Self {
                previous_int,
                previous_term,
            })
        }
        #[cfg(windows)]
        {
            let _ = ON_CANCEL_WIN.set(on_cancel);
            if unsafe { SetConsoleCtrlHandler(Some(windows_handler), 1) } == 0 {
                return Err("install console control handler failed".into());
            }
            Ok(Self {})
        }
    }
}

impl Drop for InterruptGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            signal(SIGINT, self.previous_int);
            signal(SIGTERM, self.previous_term);
        }
        #[cfg(windows)]
        unsafe {
            SetConsoleCtrlHandler(Some(windows_handler), 0);
        }
    }
}

/// Open a regular file for reading without following a final symlink and
/// without blocking on FIFOs.
pub fn open_no_follow(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // open(2) flag values are per-kernel-ABI and per-arch: use libc.
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    options.open(path)
}

/// The real path of an already-open file, used to confirm it did not escape a
/// root directory through a race between check and open.
pub fn opened_path(file: &File, requested: &Path) -> std::io::Result<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        let _ = requested;
        std::fs::canonicalize(format!("/proc/self/fd/{}", file.as_raw_fd()))
    }
    #[cfg(not(target_os = "linux"))]
    {
        // No portable fd-to-path lookup. The no-follow open already refused a
        // final symlink, so canonicalizing the requested path is equivalent.
        let _ = file;
        std::fs::canonicalize(requested)
    }
}

/// Per-user runtime directory for sockets, pids, and scratch state.
pub fn runtime_dir() -> PathBuf {
    #[cfg(unix)]
    {
        std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
    }
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA")
            .map(|d| PathBuf::from(d).join("hwatu").join("run"))
            .unwrap_or_else(std::env::temp_dir)
    }
}

/// Per-user config directory root (the parent of `hwatu/`).
pub fn config_root() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(d) = std::env::var_os("APPDATA") {
            return PathBuf::from(d);
        }
    }
    if let Some(d) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(d);
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join(".config")
}
