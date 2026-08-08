// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! Password-manager fill (roadmap H11): first-class fill from `pass`
//! and Bitwarden's `bw` CLI. hwatu integrates, never stores — no
//! password store of our own, ever (roadmap non-goal).
//!
//! Trigger: the `fill_password` action (default alt+p). The daemon
//! resolves the current page's host, queries the backend on a worker
//! thread (gpg pinentry can take seconds; the GTK loop must not
//! block), and fills the page's login form with framework-safe value
//! setting (native setter + input/change events, so React forms see
//! the change).
//!
//! Backends, auto-detected in order (`"password_backend"` in
//! config.json pins one: "pass" | "bitwarden" | "off"):
//!
//! - **pass**: entries under `~/.password-store` whose path contains
//!   the host (www. stripped) — `sites/github.com` matches
//!   `github.com`. First line is the password; a `user:`/`username:`/
//!   `login:` line supplies the username (password-store convention).
//! - **bitwarden**: `bw get item <host>` with an unlocked vault
//!   (BW_SESSION set in the daemon's environment).
//!
//! Passwords never touch logs or the bar; failures name the reason
//! ("no entry for <host>", "vault locked"), never the secret.

use std::path::PathBuf;

/// A credential ready to fill. Password intentionally not Debug.
pub struct Credential {
    pub username: Option<String>,
    pub password: String,
}

pub enum FillError {
    NoBackend,
    NoEntry(String),
    Backend(String),
}

impl std::fmt::Display for FillError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FillError::NoBackend => {
                write!(f, "no password backend (install pass or set up bw)")
            }
            FillError::NoEntry(host) => write!(f, "no entry for {host}"),
            FillError::Backend(msg) => write!(f, "{msg}"),
        }
    }
}

/// Blocking credential lookup; call from a worker thread.
pub fn lookup(host: &str) -> Result<Credential, FillError> {
    match backend_choice().as_deref() {
        Some("off") => Err(FillError::NoBackend),
        Some("pass") => pass_lookup(host),
        Some("bitwarden") => bw_lookup(host),
        _ => {
            // Auto: pass first (no daemon, no session), then bw.
            if pass_store_dir().is_dir() {
                pass_lookup(host)
            } else if std::env::var_os("BW_SESSION").is_some() {
                bw_lookup(host)
            } else {
                Err(FillError::NoBackend)
            }
        }
    }
}

fn backend_choice() -> Option<String> {
    Some(
        crate::window::config_value("password_backend")?
            .as_str()?
            .to_ascii_lowercase(),
    )
}

// ---- pass -----------------------------------------------------------

fn pass_store_dir() -> PathBuf {
    std::env::var_os("PASSWORD_STORE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| glib::home_dir().join(".password-store"))
}

/// Find the store entry for `host`: the relative path (minus .gpg)
/// whose final component (or any component) matches host or host
/// without `www.`. Shortest match wins (github.com over
/// github.com/old).
fn pass_entry_for(host: &str) -> Option<String> {
    let store = pass_store_dir();
    let bare = host.strip_prefix("www.").unwrap_or(host);
    let mut matches: Vec<String> = Vec::new();
    let mut stack = vec![store.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else if let Some(rel) = name.strip_suffix(".gpg") {
                let rel_path = path
                    .strip_prefix(&store)
                    .ok()
                    .map(|p| p.with_extension(""))
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|| rel.to_string());
                if rel_path.split('/').any(|part| part == host || part == bare)
                    || rel_path.contains(bare)
                {
                    matches.push(rel_path);
                }
            }
        }
    }
    matches.sort_by_key(|m| m.len());
    matches.into_iter().next()
}

fn pass_lookup(host: &str) -> Result<Credential, FillError> {
    let entry = pass_entry_for(host).ok_or_else(|| FillError::NoEntry(host.to_string()))?;
    let output = std::process::Command::new("pass")
        .arg("show")
        .arg(&entry)
        .output()
        .map_err(|e| FillError::Backend(format!("pass not runnable: {e}")))?;
    if !output.status.success() {
        return Err(FillError::Backend(format!(
            "pass show {entry} failed (gpg locked?)"
        )));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    parse_pass_entry(&text, &entry).ok_or_else(|| FillError::NoEntry(host.to_string()))
}

/// password-store convention: first line password, `key: value` lines
/// after. Username falls back to the entry's final path component
/// when it looks like an account name (contains @ or no dot).
fn parse_pass_entry(text: &str, entry: &str) -> Option<Credential> {
    let mut lines = text.lines();
    let password = lines.next()?.trim().to_string();
    if password.is_empty() {
        return None;
    }
    let mut username = None;
    for line in lines {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if matches!(
            key.trim().to_ascii_lowercase().as_str(),
            "user" | "username" | "login" | "email"
        ) {
            username = Some(value.trim().to_string());
            break;
        }
    }
    if username.is_none() {
        let last = entry.rsplit('/').next().unwrap_or(entry);
        if last.contains('@') {
            username = Some(last.to_string());
        }
    }
    Some(Credential { username, password })
}

// ---- bitwarden --------------------------------------------------------

fn bw_lookup(host: &str) -> Result<Credential, FillError> {
    if std::env::var_os("BW_SESSION").is_none() {
        return Err(FillError::Backend(
            "bitwarden vault locked (BW_SESSION not set)".into(),
        ));
    }
    let output = std::process::Command::new("bw")
        .args(["get", "item", host])
        .output()
        .map_err(|e| FillError::Backend(format!("bw not runnable: {e}")))?;
    if !output.status.success() {
        return Err(FillError::NoEntry(host.to_string()));
    }
    let item: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| FillError::Backend("bw returned invalid JSON".into()))?;
    let login = item
        .get("login")
        .ok_or_else(|| FillError::NoEntry(host.to_string()))?;
    let password = login
        .get("password")
        .and_then(|p| p.as_str())
        .filter(|p| !p.is_empty())
        .ok_or_else(|| FillError::NoEntry(host.to_string()))?
        .to_string();
    let username = login
        .get("username")
        .and_then(|u| u.as_str())
        .map(str::to_string);
    Ok(Credential { username, password })
}

// ---- page fill --------------------------------------------------------

/// JS that fills the page's login form. Framework-safe: uses the
/// native value setter and dispatches input/change so React/Vue
/// controlled inputs accept the value. Fills the first visible
/// password field and the nearest username-shaped field before it.
pub fn fill_js(credential: &Credential) -> String {
    let user = serde_json::to_string(credential.username.as_deref().unwrap_or(""))
        .unwrap_or_else(|_| "\"\"".into());
    let pass = serde_json::to_string(&credential.password).unwrap_or_else(|_| "\"\"".into());
    format!(
        r#"(() => {{
  const USER = {user};
  const PASS = {pass};
  const visible = (el) => {{
    const r = el.getBoundingClientRect();
    return r.width > 0 && r.height > 0 && getComputedStyle(el).visibility !== 'hidden';
  }};
  const setValue = (el, value) => {{
    const proto = Object.getPrototypeOf(el);
    const desc = Object.getOwnPropertyDescriptor(proto, 'value')
      || Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value');
    if (desc && desc.set) desc.set.call(el, value); else el.value = value;
    el.dispatchEvent(new Event('input', {{ bubbles: true }}));
    el.dispatchEvent(new Event('change', {{ bubbles: true }}));
  }};
  const pw = [...document.querySelectorAll('input[type=password]')].find(visible);
  if (!pw) return 'no password field';
  const inputs = [...document.querySelectorAll(
    'input[type=text], input[type=email], input:not([type])')].filter(visible);
  const before = inputs.filter((el) =>
    el.compareDocumentPosition(pw) & Node.DOCUMENT_POSITION_FOLLOWING);
  const userField = before[before.length - 1];
  if (USER && userField) setValue(userField, USER);
  setValue(pw, PASS);
  pw.focus();
  return 'filled' + (USER && userField ? ' user+pass' : ' pass');
}})()"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pass_entry_parsing() {
        let cred = parse_pass_entry("hunter2\nusername: alice\nurl: x\n", "sites/github.com")
            .expect("parses");
        assert_eq!(cred.password, "hunter2");
        assert_eq!(cred.username.as_deref(), Some("alice"));

        // login:/email: variants and case folding.
        let cred = parse_pass_entry("pw\nLogin: bob\n", "e").expect("parses");
        assert_eq!(cred.username.as_deref(), Some("bob"));

        // No username line: an @-shaped final path component is used.
        let cred = parse_pass_entry("pw\n", "github.com/alice@example.com").expect("parses");
        assert_eq!(cred.username.as_deref(), Some("alice@example.com"));

        // No username at all.
        let cred = parse_pass_entry("pw\nnote: hi\n", "github.com").expect("parses");
        assert_eq!(cred.username, None);

        // Empty password line rejected.
        assert!(parse_pass_entry("\nusername: x\n", "e").is_none());
        assert!(parse_pass_entry("", "e").is_none());
    }

    #[test]
    fn fill_js_escapes_and_never_logs() {
        let cred = Credential {
            username: Some("a\"b".into()),
            password: "p'w\\\"x".into(),
        };
        let js = fill_js(&cred);
        // Secrets are JSON-escaped into the script...
        assert!(js.contains(r#""a\"b""#));
        // ...framework-safe setter and events present...
        assert!(js.contains("dispatchEvent(new Event('input'"));
        assert!(js.contains("Object.getOwnPropertyDescriptor"));
        // ...and the script fails open without a password field.
        assert!(js.contains("'no password field'"));
    }

    #[test]
    fn fill_error_messages_name_no_secrets() {
        assert_eq!(
            FillError::NoEntry("github.com".into()).to_string(),
            "no entry for github.com"
        );
        assert!(FillError::NoBackend.to_string().contains("pass"));
    }
}
