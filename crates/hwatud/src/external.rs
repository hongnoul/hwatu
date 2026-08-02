// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! Open compatibility-blocked pages in an installed non-WebKit browser.
//!
//! Cloudflare explicitly gives embedded/custom WebKit environments limited
//! support.  Current Turnstile challenges reject WebKitGTK's privacy-preserving
//! WebGL fingerprint, so retrying inside the same view cannot recover.  Keep
//! the fallback explicit (the user answers a y/n prompt) and launch a browser
//! with a different engine instead of weakening WebKit's fingerprinting
//! protections or pretending that the challenge succeeded.

use std::io;
use std::process::{Command, Stdio};

const FALLBACKS: &[&str] = &[
    "helium",
    "firefox",
    "google-chrome-stable",
    "google-chrome",
    "chromium",
    "chromium-browser",
    "brave-browser",
];

fn candidates() -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(browser) = std::env::var("HWATU_EXTERNAL_BROWSER") {
        let browser = browser.trim();
        if !browser.is_empty() {
            out.push(browser.to_string());
        }
    }
    out.extend(FALLBACKS.iter().map(|name| (*name).to_string()));
    out.dedup();
    out
}

pub fn open(uri: &str) -> io::Result<String> {
    let mut last_not_found = None;
    for browser in candidates() {
        match Command::new(&browser)
            .arg(uri)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(_) => return Ok(browser),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                last_not_found = Some(error);
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_not_found.unwrap_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "no external browser is installed")
    }))
}

#[cfg(test)]
mod tests {
    use super::FALLBACKS;

    #[test]
    fn fallbacks_are_non_webkit_browsers() {
        assert!(FALLBACKS.contains(&"firefox"));
        assert!(FALLBACKS.contains(&"chromium"));
        assert!(!FALLBACKS.contains(&"epiphany"));
        assert!(!FALLBACKS.contains(&"hwatu"));
    }
}
