// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! Cross-client onboarding: `hwatu doctor`, `hwatu setup`, `hwatu demo`.
//!
//! Design rules:
//! - `setup` with no `--client` only *detects and recommends*; it never
//!   writes anything.
//! - `setup --client <x>` previews the exact target file and action
//!   before touching it, applies idempotently, and preserves unrelated
//!   JSON in shared config files.
//! - `--undo` removes only the hwatu registration; empty files/dirs that
//!   the registration plausibly created are removed cautiously
//!   (non-recursive, only when empty).
//! - No dependencies beyond `serde_json`.

use hwatu_ipc::{OpenMode, Request, Response};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn run(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("doctor") => doctor(),
        Some("setup") => setup(&args[1..]),
        Some("demo") => demo(&args[1..]),
        _ => {
            eprintln!("hwatu: internal: onboarding called without subcommand");
            2
        }
    }
}

// ===================================================================
// doctor
// ===================================================================

fn doctor() -> i32 {
    let mut failures = 0u32;
    let mut check = |name: &str, result: Result<String, String>| match result {
        Ok(msg) => println!("ok    {name}: {msg}"),
        Err(msg) => {
            println!("FAIL  {name}: {msg}");
            failures += 1;
        }
    };

    // 1. Binary and PATH.
    check("binary", {
        match std::env::current_exe() {
            Ok(exe) => {
                let on_path = which("hwatu").is_some();
                let path_note = if on_path {
                    "on PATH"
                } else {
                    "NOT on PATH (add its directory to PATH for agent clients)"
                };
                Ok(format!("{} ({path_note})", exe.display()))
            }
            Err(e) => Err(format!("cannot resolve current executable: {e}")),
        }
    });

    // 2. WebKitGTK library. The daemon links it; the thin client does
    // not, so probe the dynamic-linker cache and common lib dirs.
    check("webkit", webkit_check());

    // 3. Daemon ping (spawns the daemon if needed, like normal use).
    let daemon_ok = match send(&Request::Ping) {
        Ok(Response::Ok { value, .. }) => {
            let build = value
                .as_ref()
                .and_then(|v| v.get("build"))
                .and_then(|b| b.as_str())
                .unwrap_or("?")
                .to_string();
            check("daemon", Ok(format!("ping ok (build {build})")));
            true
        }
        Ok(Response::Err { message }) => {
            check("daemon", Err(format!("ping refused: {message}")));
            false
        }
        Err(e) => {
            check("daemon", Err(format!("unreachable: {e}")));
            false
        }
    };

    // 4. Headless rendered smoke test, with cleanup.
    if daemon_ok {
        check("render", smoke_test());
    } else {
        check("render", Err("skipped: daemon unreachable".into()));
    }

    if failures == 0 {
        println!("doctor: all checks passed");
        0
    } else {
        println!("doctor: {failures} check(s) failed");
        1
    }
}

fn webkit_check() -> Result<String, String> {
    const NEEDLES: &[&str] = &["libwebkitgtk-6.0.so"];
    // Dynamic-linker cache first: authoritative on glibc systems.
    if let Ok(out) = Command::new("ldconfig").arg("-p").output() {
        let text = String::from_utf8_lossy(&out.stdout);
        for needle in NEEDLES {
            if let Some(line) = text.lines().find(|l| l.contains(needle)) {
                return Ok(line.trim().to_string());
            }
        }
    }
    // Fall back to scanning common library directories.
    for dir in [
        "/usr/lib",
        "/usr/lib64",
        "/usr/lib/x86_64-linux-gnu",
        "/usr/lib/aarch64-linux-gnu",
        "/usr/local/lib",
    ] {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if NEEDLES.iter().any(|n| name.starts_with(n)) {
                    return Ok(format!("{}/{name}", dir));
                }
            }
        }
    }
    Err("WebKitGTK (libwebkitgtk-6.0) not found; install your distro's webkitgtk package".into())
}

/// Open a headless window on a data: URL, verify the DOM and rendered pixels,
/// and close the window/remove the screenshot regardless of outcome.
fn smoke_test() -> Result<String, String> {
    let url = "about:blank";
    let id = match send(&Request::Open {
        url: Some(url.into()),
        app_id: None,
        mode: OpenMode::Headless,
    }) {
        Ok(Response::Ok {
            window: Some(w), ..
        }) => w.id,
        Ok(Response::Ok { .. }) => return Err("open returned no window".into()),
        Ok(Response::Err { message }) => return Err(format!("open failed: {message}")),
        Err(e) => return Err(format!("open failed: {e}")),
    };
    let shot = std::env::temp_dir().join(format!("hwatu-doctor-{}.png", std::process::id()));
    let result = (|| {
        let _ = send(&Request::WaitLoad {
            id: Some(id),
            until: Default::default(),
            timeout_ms: Some(10_000),
        });
        match send(&Request::Eval {
            id: Some(id),
            js: "document.title='hwatu-doctor'; document.body.innerHTML='<h1>hwatu doctor smoke test</h1>'; return document.title".into(),
            timeout_ms: Some(10_000),
        }) {
            Ok(Response::Ok { value: Some(v), .. }) if v.as_str() == Some("hwatu-doctor") => Ok(()),
            Ok(Response::Ok { value, .. }) => Err(format!("unexpected eval result: {value:?}")),
            Ok(Response::Err { message }) => Err(format!("eval failed: {message}")),
            Err(e) => Err(format!("eval failed: {e}")),
        }?;
        match send(&Request::Screenshot {
            id: Some(id),
            path: Some(shot.to_string_lossy().into_owned()),
            full: false,
        }) {
            Ok(Response::Ok { .. }) => match fs::metadata(&shot) {
                Ok(meta) if meta.len() > 0 => {
                    Ok("headless window rendered, evaluated JS, and captured pixels".to_string())
                }
                Ok(_) => Err("screenshot was empty".into()),
                Err(e) => Err(format!("screenshot was not written: {e}")),
            },
            Ok(Response::Err { message }) => Err(format!("screenshot failed: {message}")),
            other => Err(format!("screenshot failed: {other:?}")),
        }
    })();
    // Cleanup: never leave the smoke-test window behind.
    let _ = send(&Request::Close { id });
    let _ = fs::remove_file(shot);
    result
}

// ===================================================================
// setup
// ===================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Client {
    Claude,
    Cursor,
    Generic,
    Jcode,
}

impl Client {
    fn parse(s: &str) -> Option<Client> {
        match s {
            "claude" => Some(Client::Claude),
            "cursor" => Some(Client::Cursor),
            "generic" => Some(Client::Generic),
            "jcode" => Some(Client::Jcode),
            _ => None,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Client::Claude => "claude",
            Client::Cursor => "cursor",
            Client::Generic => "generic",
            Client::Jcode => "jcode",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Project,
    User,
}

const SETUP_USAGE: &str =
    "usage: hwatu setup [--client claude|cursor|generic|jcode] [--scope project|user] \
     [--dry-run] [--undo]";

fn setup(args: &[String]) -> i32 {
    let mut client: Option<Client> = None;
    let mut scope = Scope::Project;
    let mut dry_run = false;
    let mut undo = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--client" => match it.next().map(|v| Client::parse(v)) {
                Some(Some(c)) => client = Some(c),
                _ => {
                    eprintln!("{SETUP_USAGE}");
                    return 2;
                }
            },
            "--scope" => match it.next().map(String::as_str) {
                Some("project") => scope = Scope::Project,
                Some("user") => scope = Scope::User,
                _ => {
                    eprintln!("{SETUP_USAGE}");
                    return 2;
                }
            },
            "--dry-run" => dry_run = true,
            "--undo" => undo = true,
            other => {
                eprintln!("unknown argument {other:?}\n{SETUP_USAGE}");
                return 2;
            }
        }
    }

    let Some(client) = client else {
        // Bare `setup`: detect and recommend only. Never configure.
        return setup_detect();
    };

    if client == Client::Jcode {
        println!("jcode drives hwatu natively (its browser tool shells out to `hwatu`).");
        println!("No MCP configuration is needed; just keep `hwatu` on PATH.");
        return 0;
    }

    let home = match home_dir() {
        Some(h) => h,
        None => {
            eprintln!("hwatu: cannot determine home directory");
            return 1;
        }
    };
    let project = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let path = config_path(client, scope, &project, &home);
    let action = if undo {
        "remove hwatu from"
    } else {
        "register hwatu in"
    };

    // Claude has a native registration CLI; prefer it when present.
    if client == Client::Claude {
        if let Some(claude) = which("claude") {
            let scope_arg = match scope {
                Scope::Project => "project",
                Scope::User => "user",
            };
            let exe = hwatu_command();
            let argv: Vec<String> = if undo {
                vec![
                    "mcp".into(),
                    "remove".into(),
                    "--scope".into(),
                    scope_arg.into(),
                    "hwatu".into(),
                ]
            } else {
                vec![
                    "mcp".into(),
                    "add".into(),
                    "--scope".into(),
                    scope_arg.into(),
                    "hwatu".into(),
                    "--".into(),
                    exe,
                    "mcp".into(),
                ]
            };
            println!("target: native `claude mcp` CLI ({})", claude.display());
            println!("action: claude {}", argv.join(" "));
            if dry_run {
                println!("dry-run: not executed");
                return 0;
            }
            return match Command::new(&claude).args(&argv).status() {
                Ok(s) if s.success() => 0,
                Ok(s) => {
                    eprintln!("hwatu: claude mcp exited with {s}");
                    1
                }
                Err(e) => {
                    eprintln!("hwatu: failed to run claude: {e}");
                    1
                }
            };
        }
    }

    // Safe JSON config write path.
    println!("client: {}  scope: {:?}", client.name(), scope);
    println!("target: {}", path.display());
    println!("action: {action} \"mcpServers\" (unrelated JSON preserved)");
    if dry_run {
        println!("dry-run: no changes made");
        return 0;
    }
    let outcome = if undo {
        remove_registration(&path).map(|r| match r {
            RemovalOutcome::Registration => "removed hwatu registration".to_string(),
            RemovalOutcome::ConfigFile => {
                "removed hwatu registration and now-empty config file".to_string()
            }
            RemovalOutcome::NotConfigured => "hwatu was not registered; nothing to do".to_string(),
            RemovalOutcome::FileMissing => "config file does not exist; nothing to do".to_string(),
        })
    } else {
        apply_registration(&path, &hwatu_command()).map(|a| match a {
            Applied::Created => format!("created {} with hwatu registration", path.display()),
            Applied::Updated => "added hwatu registration".to_string(),
            Applied::AlreadyConfigured => "already configured; no changes made".to_string(),
        })
    };
    match outcome {
        Ok(msg) => {
            println!("done: {msg}");
            0
        }
        Err(e) => {
            eprintln!("hwatu: {e}");
            1
        }
    }
}

fn setup_detect() -> i32 {
    println!("hwatu setup: detection only; nothing was modified.");
    println!("Detected clients:");
    let home = home_dir();
    let has = |p: &str| home.as_ref().map(|h| h.join(p).exists()).unwrap_or(false);
    let mut any = false;
    if which("claude").is_some() || has(".claude") {
        any = true;
        println!("  claude   -> hwatu setup --client claude [--scope project|user]");
    }
    if which("cursor").is_some() || has(".cursor") || Path::new(".cursor").exists() {
        any = true;
        println!("  cursor   -> hwatu setup --client cursor [--scope project|user]");
    }
    if which("jcode").is_some() || has(".jcode") {
        any = true;
        println!("  jcode    -> native support, no config needed (keep `hwatu` on PATH)");
    }
    for (binary, label) in [
        ("codex", "Codex"),
        ("gemini", "Gemini CLI"),
        ("opencode", "OpenCode"),
    ] {
        if which(binary).is_some() {
            any = true;
            println!(
                "  {binary:<8} -> {label}: use its MCP UI with `hwatu mcp`, or the CLI fallback"
            );
        }
    }
    if !any {
        println!("  (none of claude/cursor/jcode detected)");
    }
    println!("  generic  -> hwatu setup --client generic [--scope project|user]");
    println!("Add --dry-run to preview, --undo to remove a registration.");
    0
}

/// The config file a client/scope pair uses. Pure so tests can point
/// it at temp dirs.
pub fn config_path(client: Client, scope: Scope, project: &Path, home: &Path) -> PathBuf {
    match (client, scope) {
        (Client::Claude, Scope::Project) => project.join(".mcp.json"),
        (Client::Claude, Scope::User) => home.join(".claude.json"),
        (Client::Cursor, Scope::Project) => project.join(".cursor").join("mcp.json"),
        (Client::Cursor, Scope::User) => home.join(".cursor").join("mcp.json"),
        (Client::Generic, Scope::Project) => project.join(".mcp.json"),
        (Client::Generic, Scope::User) => home.join(".config").join("hwatu").join("mcp.json"),
        (Client::Jcode, _) => PathBuf::new(), // native; never written
    }
}

/// Prefer the resolved absolute path so registrations work even when
/// hwatu is not on PATH; fall back to the bare name.
fn hwatu_command() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(String::from))
        .unwrap_or_else(|| "hwatu".into())
}

fn server_entry(command: &str) -> serde_json::Value {
    serde_json::json!({ "command": command, "args": ["mcp"] })
}

#[derive(Debug, PartialEq, Eq)]
pub enum Applied {
    Created,
    Updated,
    AlreadyConfigured,
}

/// Idempotently register hwatu under `mcpServers.hwatu`, preserving
/// every unrelated key in the file. Refuses to touch a file it cannot
/// parse, so a malformed config is never clobbered.
pub fn apply_registration(path: &Path, command: &str) -> Result<Applied, String> {
    let (mut root, existed) = match fs::read_to_string(path) {
        Ok(raw) => {
            let v: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
                format!(
                    "{} is not valid JSON ({e}); fix or remove it first",
                    path.display()
                )
            })?;
            if !v.is_object() {
                return Err(format!(
                    "{} is not a JSON object; refusing to modify",
                    path.display()
                ));
            }
            (v, true)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (serde_json::json!({}), false),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };

    let entry = server_entry(command);
    let servers = root
        .as_object_mut()
        .expect("checked object above")
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));
    let servers = servers.as_object_mut().ok_or_else(|| {
        format!(
            "{}: \"mcpServers\" is not an object; refusing to modify",
            path.display()
        )
    })?;
    if servers.get("hwatu") == Some(&entry) {
        return Ok(Applied::AlreadyConfigured);
    }
    servers.insert("hwatu".into(), entry);

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
    }
    write_json(path, &root)?;
    Ok(if existed {
        Applied::Updated
    } else {
        Applied::Created
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum RemovalOutcome {
    Registration,
    ConfigFile,
    NotConfigured,
    FileMissing,
}

/// Remove only the hwatu registration. Drops `mcpServers` if it
/// becomes empty, deletes the file only if the whole document becomes
/// an empty object, and then removes the parent dir only when empty
/// and clearly config-related (`.cursor`, `hwatu`).
pub fn remove_registration(path: &Path) -> Result<RemovalOutcome, String> {
    let raw = match fs::read_to_string(path) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RemovalOutcome::FileMissing);
        }
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let mut root: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
        format!(
            "{} is not valid JSON ({e}); refusing to modify",
            path.display()
        )
    })?;
    let Some(obj) = root.as_object_mut() else {
        return Err(format!(
            "{} is not a JSON object; refusing to modify",
            path.display()
        ));
    };
    let removed = obj
        .get_mut("mcpServers")
        .and_then(|s| s.as_object_mut())
        .map(|s| s.remove("hwatu").is_some())
        .unwrap_or(false);
    if !removed {
        return Ok(RemovalOutcome::NotConfigured);
    }
    let servers_empty = obj
        .get("mcpServers")
        .and_then(|s| s.as_object())
        .map(|s| s.is_empty())
        .unwrap_or(false);
    if servers_empty {
        obj.remove("mcpServers");
    }
    if obj.is_empty() {
        fs::remove_file(path).map_err(|e| format!("cannot remove {}: {e}", path.display()))?;
        if let Some(parent) = path.parent() {
            let name = parent.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name == ".cursor" || name == "hwatu" {
                let _ = fs::remove_dir(parent); // fails (kept) when non-empty
            }
        }
        return Ok(RemovalOutcome::ConfigFile);
    }
    write_json(path, &root)?;
    Ok(RemovalOutcome::Registration)
}

fn write_json(path: &Path, value: &serde_json::Value) -> Result<(), String> {
    let mut text = serde_json::to_string_pretty(value).expect("serialize config");
    text.push('\n');
    fs::write(path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

// ===================================================================
// demo
// ===================================================================

const DEMO_USAGE: &str = "usage: hwatu demo [url] [--focus]";

fn demo(args: &[String]) -> i32 {
    let mut url = "https://example.com".to_string();
    let mut focus = false;
    for arg in args {
        match arg.as_str() {
            "--focus" => focus = true,
            a if a.starts_with("--") => {
                eprintln!("unknown flag {a:?}\n{DEMO_USAGE}");
                return 2;
            }
            a => url = a.to_string(),
        }
    }
    let mode = if focus {
        OpenMode::Normal
    } else {
        OpenMode::Headless
    };

    println!(
        "demo: opening {url} ({} mode)",
        if focus { "focused" } else { "headless" }
    );
    let id = match send(&Request::Open {
        url: Some(url.clone()),
        app_id: None,
        mode,
    }) {
        Ok(Response::Ok {
            window: Some(w), ..
        }) => w.id,
        Ok(Response::Ok { .. }) => {
            eprintln!("hwatu: open returned no window");
            return 1;
        }
        Ok(Response::Err { message }) => {
            eprintln!("hwatu: open failed: {message}");
            return 1;
        }
        Err(e) => {
            eprintln!("hwatu: cannot reach daemon: {e}");
            return 1;
        }
    };
    println!("demo: window {id} open");
    let _ = send(&Request::WaitLoad {
        id: Some(id),
        until: Default::default(),
        timeout_ms: Some(15_000),
    });

    let mut ok = true;
    match send(&Request::Snapshot {
        id: Some(id),
        diff: false,
        timeout_ms: Some(15_000),
    }) {
        Ok(Response::Ok { value: Some(v), .. }) => {
            let title = v.get("title").and_then(|t| t.as_str()).unwrap_or("?");
            println!("demo: snapshot ok (title: {title})");
        }
        other => {
            ok = false;
            eprintln!("hwatu: snapshot failed: {other:?}");
        }
    }
    match send(&Request::Screenshot {
        id: Some(id),
        path: None,
        full: false,
    }) {
        Ok(Response::Ok { path: Some(p), .. }) => println!("demo: screenshot -> {p}"),
        other => {
            ok = false;
            eprintln!("hwatu: screenshot failed: {other:?}");
        }
    }

    if focus {
        println!("demo: handoff: window {id} left open and focused; close with: hwatu close {id}");
    } else {
        match send(&Request::Close { id }) {
            Ok(Response::Ok { .. }) => println!("demo: cleaned up window {id}"),
            _ => eprintln!("hwatu: could not close window {id}; close manually: hwatu close {id}"),
        }
    }
    if ok {
        println!("demo: success");
        0
    } else {
        1
    }
}

// ===================================================================
// shared helpers
// ===================================================================

fn send(request: &Request) -> Result<Response, String> {
    use std::io::{BufRead, BufReader, Write};
    let mut stream = crate::connect_or_spawn().map_err(|e| e.to_string())?;
    let mut payload = serde_json::to_vec(request).expect("serialize request");
    payload.push(b'\n');
    stream.write_all(&payload).map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    serde_json::from_str(line.trim()).map_err(|e| format!("bad response: {e}"))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn tmpdir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hwatu-onboarding-{name}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn read_json(path: &Path) -> serde_json::Value {
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn apply_creates_file_and_dirs() {
        let dir = tmpdir("create");
        let path = dir.join(".cursor").join("mcp.json");
        assert_eq!(
            apply_registration(&path, "/bin/hwatu").unwrap(),
            Applied::Created
        );
        let json = read_json(&path);
        assert_eq!(json["mcpServers"]["hwatu"]["command"], "/bin/hwatu");
        assert_eq!(json["mcpServers"]["hwatu"]["args"][0], "mcp");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn apply_is_idempotent() {
        let dir = tmpdir("idem");
        let path = dir.join("mcp.json");
        assert_eq!(
            apply_registration(&path, "hwatu").unwrap(),
            Applied::Created
        );
        let first = fs::read_to_string(&path).unwrap();
        assert_eq!(
            apply_registration(&path, "hwatu").unwrap(),
            Applied::AlreadyConfigured
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), first);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn apply_preserves_unrelated_json() {
        let dir = tmpdir("preserve");
        let path = dir.join("mcp.json");
        fs::write(
            &path,
            r#"{"theme":"dark","mcpServers":{"other":{"command":"other-tool"}}}"#,
        )
        .unwrap();
        assert_eq!(
            apply_registration(&path, "hwatu").unwrap(),
            Applied::Updated
        );
        let json = read_json(&path);
        assert_eq!(json["theme"], "dark");
        assert_eq!(json["mcpServers"]["other"]["command"], "other-tool");
        assert_eq!(json["mcpServers"]["hwatu"]["command"], "hwatu");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn apply_updates_stale_command() {
        let dir = tmpdir("stale");
        let path = dir.join("mcp.json");
        assert_eq!(
            apply_registration(&path, "/old/hwatu").unwrap(),
            Applied::Created
        );
        assert_eq!(
            apply_registration(&path, "/new/hwatu").unwrap(),
            Applied::Updated
        );
        assert_eq!(
            read_json(&path)["mcpServers"]["hwatu"]["command"],
            "/new/hwatu"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn apply_refuses_invalid_json() {
        let dir = tmpdir("invalid");
        let path = dir.join("mcp.json");
        fs::write(&path, "{not json").unwrap();
        assert!(apply_registration(&path, "hwatu").is_err());
        // Original content untouched.
        assert_eq!(fs::read_to_string(&path).unwrap(), "{not json");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn undo_removes_only_hwatu() {
        let dir = tmpdir("undo-partial");
        let path = dir.join("mcp.json");
        fs::write(
            &path,
            r#"{"theme":"dark","mcpServers":{"other":{"command":"x"},"hwatu":{"command":"hwatu","args":["mcp"]}}}"#,
        )
        .unwrap();
        assert_eq!(
            remove_registration(&path).unwrap(),
            RemovalOutcome::Registration
        );
        let json = read_json(&path);
        assert_eq!(json["theme"], "dark");
        assert_eq!(json["mcpServers"]["other"]["command"], "x");
        assert!(json["mcpServers"].get("hwatu").is_none());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn undo_removes_created_file_and_dir() {
        let dir = tmpdir("undo-full");
        let path = dir.join(".cursor").join("mcp.json");
        apply_registration(&path, "hwatu").unwrap();
        assert_eq!(
            remove_registration(&path).unwrap(),
            RemovalOutcome::ConfigFile
        );
        assert!(!path.exists());
        assert!(!dir.join(".cursor").exists(), "empty created dir removed");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn undo_keeps_nonempty_dir() {
        let dir = tmpdir("undo-dir-kept");
        let cursor = dir.join(".cursor");
        fs::create_dir_all(&cursor).unwrap();
        fs::write(cursor.join("settings.json"), "{}").unwrap();
        let path = cursor.join("mcp.json");
        apply_registration(&path, "hwatu").unwrap();
        assert_eq!(
            remove_registration(&path).unwrap(),
            RemovalOutcome::ConfigFile
        );
        assert!(cursor.exists(), "dir with unrelated files preserved");
        assert!(cursor.join("settings.json").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn undo_noops_gracefully() {
        let dir = tmpdir("undo-noop");
        let missing = dir.join("mcp.json");
        assert_eq!(
            remove_registration(&missing).unwrap(),
            RemovalOutcome::FileMissing
        );
        fs::write(&missing, r#"{"mcpServers":{"other":{}}}"#).unwrap();
        assert_eq!(
            remove_registration(&missing).unwrap(),
            RemovalOutcome::NotConfigured
        );
        // Unrelated registration untouched.
        assert_eq!(
            read_json(&missing)["mcpServers"]["other"],
            serde_json::json!({})
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn undo_refuses_invalid_json() {
        let dir = tmpdir("undo-invalid");
        let path = dir.join("mcp.json");
        fs::write(&path, "[1,2").unwrap();
        assert!(remove_registration(&path).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "[1,2");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn config_paths_per_client_scope() {
        let p = Path::new("/proj");
        let h = Path::new("/home/u");
        assert_eq!(
            config_path(Client::Claude, Scope::Project, p, h),
            PathBuf::from("/proj/.mcp.json")
        );
        assert_eq!(
            config_path(Client::Claude, Scope::User, p, h),
            PathBuf::from("/home/u/.claude.json")
        );
        assert_eq!(
            config_path(Client::Cursor, Scope::Project, p, h),
            PathBuf::from("/proj/.cursor/mcp.json")
        );
        assert_eq!(
            config_path(Client::Cursor, Scope::User, p, h),
            PathBuf::from("/home/u/.cursor/mcp.json")
        );
        assert_eq!(
            config_path(Client::Generic, Scope::Project, p, h),
            PathBuf::from("/proj/.mcp.json")
        );
        assert_eq!(
            config_path(Client::Generic, Scope::User, p, h),
            PathBuf::from("/home/u/.config/hwatu/mcp.json")
        );
    }
}
