// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! Embed the git commit into the binary so the daemon can report its
//! build over IPC (`ping`), letting the CLI flag daemon/client skew.
use std::process::Command;

fn main() {
    let hash = Command::new("git")
        .args(["rev-parse", "--short=9", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=HWATU_GIT_HASH={hash}");
    // Rebuild when HEAD moves so the hash never goes stale.
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs");
}
