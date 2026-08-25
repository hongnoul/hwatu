// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! `hwatu update`: self-update from GitHub releases.
//!
//! Downloads the latest release tarball for this platform, verifies the
//! sha256, and installs over the currently running binaries' directory.
//! Uses `curl`/`tar`/`sha256sum` (present on any system that ran the
//! installer) so the client keeps zero Rust dependencies.
//!
//! The running daemon keeps its old code (Unix semantics); the update
//! finishes with a restart hint unless no daemon is running.

use std::path::{Path, PathBuf};
use std::process::Command;

const REPO: &str = "hongnoul/hwatu";

pub fn run() -> i32 {
    match update() {
        Ok(()) => 0,
        Err(msg) => {
            eprintln!("hwatu: {msg}");
            1
        }
    }
}

fn update() -> Result<(), String> {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        other => {
            return Err(format!(
                "no prebuilt binaries for {other}; build from source"
            ))
        }
    };
    if std::env::consts::OS != "linux" {
        return Err("prebuilt binaries are Linux-only; build from source".into());
    }
    let artifact = format!("hwatu-linux-{arch}");

    // Install where the running binary lives, so update updates *this*
    // install (repo checkout, ~/.local/bin, or wherever).
    let install_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .ok_or("cannot locate current executable")?;

    // A package-manager install (pacman, apt, nix) owns these files. Writing
    // over them desyncs the package database and, under sudo, drops unsigned
    // binaries into a root-owned directory. Refuse and defer to the manager.
    if !dir_is_user_writable(&install_dir) {
        return Err(format!(
            "{} is not writable by this user; hwatu looks package-manager installed.\n\
             update it with your package manager instead (for example `pacman -Syu hwatu`).\n\
             do not re-run this under sudo: it would overwrite package-owned files.",
            install_dir.display()
        ));
    }

    let tmp = mktemp_dir()?;
    let _cleanup = Cleanup(tmp.clone());

    // Resolve the latest tag from the /releases/latest redirect
    // (Location: .../releases/tag/vX.Y.Z). No api.github.com, so no
    // per-IP rate limit.
    let tag = latest_tag()?;
    if tag == format!("v{}", env!("CARGO_PKG_VERSION")) {
        println!("already up to date ({tag})");
        return Ok(());
    }

    let base = format!("https://github.com/{REPO}/releases/download/{tag}/{artifact}.tar.gz");
    let pkg = tmp.join("pkg.tar.gz");
    println!("downloading {artifact} {tag}...");
    curl_download(&base, &pkg)?;

    // Checksum (best effort like the installer: skip if absent).
    let sums = tmp.join("pkg.sha256");
    if curl_download(&format!("{base}.sha256"), &sums).is_ok() {
        let expected = std::fs::read_to_string(&sums)
            .map_err(|e| format!("read checksum: {e}"))?
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_string();
        let out = Command::new("sha256sum")
            .arg(&pkg)
            .output()
            .map_err(|e| format!("sha256sum: {e}"))?;
        let actual = String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_string();
        if expected != actual {
            return Err("checksum verification failed".into());
        }
    }

    run_ok(
        Command::new("tar")
            .args(["xzf"])
            .arg(&pkg)
            .arg("-C")
            .arg(&tmp),
        "tar",
    )?;

    // Atomic per-file: write next to the target, rename over it. The
    // running daemon keeps its inode; new spawns get the new code.
    for bin in ["hwatu", "hwatud"] {
        let src = tmp.join(&artifact).join(bin);
        let dst = install_dir.join(bin);
        let staged = install_dir.join(format!(".{bin}.update"));
        std::fs::copy(&src, &staged).map_err(|e| format!("stage {bin}: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))
                .map_err(|e| format!("chmod {bin}: {e}"))?;
        }
        std::fs::rename(&staged, &dst).map_err(|e| format!("install {bin}: {e}"))?;
    }

    println!(
        "updated to {tag} in {} (was v{})",
        install_dir.display(),
        env!("CARGO_PKG_VERSION")
    );

    // Restart hint only if a daemon is actually running old code.
    if std::os::unix::net::UnixStream::connect(hwatu_ipc::socket_path()).is_ok() {
        println!("the running daemon still has the old version.");
        println!("run `hwatu quit` (closes all windows), then `hwatu` to restart on {tag}.");
    }
    Ok(())
}

fn latest_tag() -> Result<String, String> {
    let out = Command::new("curl")
        .args(["-fsSI", "-o", "/dev/null", "-w", "%{redirect_url}"])
        .arg(format!("https://github.com/{REPO}/releases/latest"))
        .output()
        .map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        return Err("cannot resolve latest release".into());
    }
    let loc = String::from_utf8_lossy(&out.stdout);
    loc.trim()
        .rsplit_once("/tag/")
        .map(|(_, tag)| tag.to_string())
        .filter(|t| !t.is_empty())
        .ok_or_else(|| format!("unexpected release redirect: {loc}"))
}

fn curl_download(url: &str, dest: &Path) -> Result<(), String> {
    let out = Command::new("curl")
        .args(["-fsSL", "-o"])
        .arg(dest)
        .arg(url)
        .output()
        .map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        return Err(format!("download failed: {url}"));
    }
    Ok(())
}

fn run_ok(cmd: &mut Command, name: &str) -> Result<(), String> {
    let status = cmd.status().map_err(|e| format!("{name}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{name} failed with {status}"))
    }
}

/// True when the current user can create files in `dir`.
///
/// Uses an actual create attempt rather than reading mode bits, so it stays
/// correct under ACLs, read-only mounts, and root (for whom mode bits lie).
fn dir_is_user_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".hwatu-write-probe-{}", std::process::id()));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

fn mktemp_dir() -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join(format!("hwatu-update-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("mktemp: {e}"))?;
    Ok(dir)
}

/// RAII temp-dir removal, also on error paths.
struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writability_probe_leaves_no_residue() {
        let dir = std::env::temp_dir().join(format!("hwatu-probe-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(dir_is_user_writable(&dir));
        let leftovers: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
        assert!(leftovers.is_empty(), "probe file must be cleaned up");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unwritable_install_dir_is_refused() {
        // Root bypasses permission bits, so the probe would succeed and this
        // assertion would not describe the guard. This module deliberately
        // carries no Rust deps, so shell out rather than pull in `libc`.
        let euid = Command::new("id")
            .arg("-u")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse::<u32>().ok());
        if euid == Some(0) {
            return;
        }
        let dir = std::env::temp_dir().join(format!("hwatu-ro-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
            assert!(
                !dir_is_user_writable(&dir),
                "a read-only directory must read as not user-writable"
            );
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
