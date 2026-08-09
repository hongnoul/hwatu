// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! hana: thin client for the hwatud browser daemon.
//!
//! `hana <url>` opens a window in ~1 IPC roundtrip. If no daemon is
//! running, it spawns one and waits for the socket.

mod clone;
mod mcp;
mod onboarding;
mod update;

use hwatu_ipc::{
    AdblockCmd, ClockAction, LoadStage, OpenMode, PressKey, Request, Response, Viewport,
};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::Command;
use std::time::{Duration, Instant};

/// Resolve daemon-facing filesystem paths against the client's working
/// directory. The daemon may be long-lived and have been spawned from a
/// different directory, so a relative path would put artifacts somewhere
/// other than where the caller expects them.
pub(crate) fn resolve_path(path: Option<String>) -> Option<String> {
    path.map(|path| {
        let path_buf = std::path::PathBuf::from(path);
        if path_buf.is_absolute() {
            return path_buf.to_string_lossy().into_owned();
        }
        std::env::current_dir()
            .map(|cwd| cwd.join(&path_buf).to_string_lossy().into_owned())
            .unwrap_or_else(|_| path_buf.to_string_lossy().into_owned())
    })
}

pub(crate) fn normalize_request_paths(request: &mut Request) {
    match request {
        Request::Screenshot { path, .. } => *path = resolve_path(path.take()),
        Request::Check {
            shot_path,
            baseline,
            heatmap,
            baseline_dir,
            ..
        } => {
            *shot_path = resolve_path(shot_path.take());
            *baseline = resolve_path(baseline.take());
            *heatmap = resolve_path(heatmap.take());
            *baseline_dir = resolve_path(baseline_dir.take());
        }
        Request::Upload { path, .. } => {
            *path = resolve_path(Some(std::mem::take(path))).expect("path is present");
        }
        Request::Diff {
            baseline, heatmap, ..
        } => {
            *baseline = resolve_path(baseline.take());
            *heatmap = resolve_path(heatmap.take());
        }
        Request::Batch { actions } => {
            for action in actions {
                normalize_request_paths(action);
            }
        }
        _ => {}
    }
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("update") {
        std::process::exit(update::run());
    }
    if args.first().map(String::as_str) == Some("mcp") {
        std::process::exit(mcp::run());
    }
    if args.first().map(String::as_str) == Some("watch") {
        std::process::exit(watch(&args[1..]));
    }
    if args.first().map(String::as_str) == Some("clone") {
        std::process::exit(clone::run(&args[1..]));
    }
    if is_onboarding_command(args.first().map(String::as_str)) {
        std::process::exit(onboarding::run(&args));
    }
    // `--json` is a client-side output flag (machine-readable `list`
    // for wofi/rofi/fuzzel pipelines), not part of the wire protocol.
    let json = {
        let before = args.len();
        args.retain(|a| a != "--json");
        args.len() != before
    };
    let request = match parse(&args) {
        Ok(r) => r,
        Err(msg) => {
            eprintln!("{msg}");
            std::process::exit(2);
        }
    };
    if matches!(request, Request::Expect { watch: true, .. }) {
        std::process::exit(expect_watch(request));
    }

    let started = Instant::now();
    let mut stream = match connect_or_spawn() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("hwatu: cannot reach daemon: {e}");
            std::process::exit(1);
        }
    };

    let mut payload = serde_json::to_vec(&request).expect("serialize request");
    payload.push(b'\n');
    if let Err(e) = stream.write_all(&payload) {
        eprintln!("hwatu: write failed: {e}");
        std::process::exit(1);
    }

    let mut line = String::new();
    let mut reader = BufReader::new(stream);
    if let Err(e) = reader.read_line(&mut line) {
        eprintln!("hwatu: read failed: {e}");
        std::process::exit(1);
    }

    match serde_json::from_str::<Response>(line.trim()) {
        Ok(Response::Ok {
            window,
            windows,
            adblock,
            value,
            path,
        }) => {
            if let Some(w) = window {
                if json {
                    println!("{}", serde_json::to_string(&w).expect("serialize window"));
                } else {
                    println!(
                        "window {} -> {} ({} ms)",
                        w.id,
                        w.url,
                        started.elapsed().as_millis()
                    );
                }
            }
            if let Some(ws) = windows {
                if json {
                    println!("{}", serde_json::to_string(&ws).expect("serialize windows"));
                } else {
                    for w in ws {
                        let flag = if w.suspended { "suspended" } else { "live" };
                        println!("{}\t{}\t{}\t{}", w.id, flag, w.url, w.title);
                    }
                }
            }
            if let Some(a) = adblock {
                let state = if a.enabled { "on" } else { "off" };
                let mut extra = String::new();
                if a.compiling {
                    extra.push_str(", compiling");
                }
                if a.updating {
                    extra.push_str(", updating lists");
                }
                println!("adblock {state}: {} rules ({}{extra})", a.rules, a.source);
            }
            if let Some(v) = value {
                // Eval results are machine-facing: always JSON.
                println!("{v}");
                // Ping is the version handshake: an old daemon serving
                // a new client (or vice versa) is the root cause behind
                // "feature X doesn't work" reports, so say it out loud.
                if matches!(request, Request::Ping) {
                    let daemon_build = v.get("build").and_then(|b| b.as_str()).unwrap_or("?");
                    let client_build = env!("HWATU_GIT_HASH");
                    if daemon_build != client_build {
                        eprintln!(
                            "hwatu: daemon build {daemon_build} != client build \
                             {client_build}; restart the daemon to match: \
                             hwatu quit && hwatu ping"
                        );
                    }
                }
            } else if matches!(request, Request::Eval { .. }) {
                // A null result serializes as `"value":null`, which
                // deserializes to `None` here. Eval always answers, so
                // print the null instead of nothing: silence would be
                // indistinguishable from a swallowed result.
                println!("null");
            }
            if let Some(p) = path {
                println!("{p}");
            }
        }
        Ok(Response::Err { message }) => {
            eprintln!("hwatu: {message}");
            // "unknown variant" means the running daemon predates this
            // CLI's protocol: the classic stale-daemon failure after an
            // upgrade. Name the fix instead of leaving agents guessing.
            if message.contains("unknown variant") {
                eprintln!(
                    "hwatu: the running daemon is older than this client \
                     (client build {}); restart it to pick up the new \
                     protocol: hwatu quit && hwatu ping",
                    env!("HWATU_GIT_HASH")
                );
            }
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("hwatu: bad response: {e} ({line:?})");
            std::process::exit(1);
        }
    }
}

/// Return whether the first argument is handled locally instead of being
/// interpreted as a URL by the browser client.
fn is_onboarding_command(command: Option<&str>) -> bool {
    matches!(command, Some("doctor") | Some("setup") | Some("demo"))
}

fn parse(args: &[String]) -> Result<Request, String> {
    let mut request = parse_with_default_mode(args, default_open_mode())?;
    normalize_request_paths(&mut request);
    Ok(request)
}

/// Coding agents mark their subprocess environment; a human's shell or
/// WM keybind has none of these. Opens from an agent default to
/// `Headless` so verification flows never appear in the WM at all,
/// while human entries keep Normal. Explicit flags always win
/// (`--focus` opts an agent back into Normal deliberately), and the
/// agent default is configurable: `HWATU_AGENT_MODE` env, then
/// `agent_mode` in `~/.config/hwatu/config.json`, then headless.
fn default_open_mode() -> OpenMode {
    const AGENT_MARKERS: &[&str] = &[
        "CLAUDECODE",    // Claude Code
        "CODEX_SANDBOX", // Codex CLI
        "CURSOR_AGENT",  // Cursor CLI
        "AGENT",         // Amp, and a de-facto generic marker
        "OPENCODE",      // opencode
        "GEMINI_CLI",    // Gemini CLI
    ];
    // jcode tool subprocesses carry various JCODE_* vars (JCODE_SCRATCH_DIR,
    // JCODE_NON_INTERACTIVE, ...) but not always JCODE_SOCKET, so treat any
    // JCODE_-prefixed var as an agent marker, except user-config knobs that
    // people export session-wide from .profile/environment.d (those would
    // make the whole desktop look like an agent and force every WM-keybind
    // launch headless).
    const JCODE_USER_CONFIG_VARS: &[&str] = &["JCODE_NO_AUTO_UPDATE", "JCODE_BING_API_KEY"];
    let from_jcode = std::env::vars_os().any(|(k, _)| {
        k.to_str()
            .map(|k| k.starts_with("JCODE_") && !JCODE_USER_CONFIG_VARS.contains(&k))
            .unwrap_or(false)
    });
    let from_agent = from_jcode || AGENT_MARKERS.iter().any(|k| std::env::var_os(k).is_some());
    let gio_launch_pid = std::env::var("GIO_LAUNCHED_DESKTOP_FILE_PID").ok();
    let user_initiated = is_current_gio_launch(
        std::env::var_os("GIO_LAUNCHED_DESKTOP_FILE").is_some(),
        gio_launch_pid.as_deref(),
        std::process::id(),
    ) || std::env::var("JCODE_OPEN_ORIGIN").as_deref() == Ok("user");
    if should_default_to_normal(user_initiated, from_agent) {
        return OpenMode::Normal;
    }
    if let Ok(v) = std::env::var("HWATU_AGENT_MODE") {
        if let Some(mode) = parse_open_mode(&v) {
            return mode;
        }
        eprintln!(
            "hwatu: ignoring invalid HWATU_AGENT_MODE={v:?} (want normal|background|headless)"
        );
    }
    config_agent_mode().unwrap_or(OpenMode::Headless)
}

/// A URL explicitly activated by a user is visible even when the application
/// that called the system opener is itself an agent UI. GIO marks conforming
/// desktop-entry launches; jcode supplies `JCODE_OPEN_ORIGIN=user` because some
/// `xdg-open` implementations execute desktop entries directly without a GIO
/// marker. Without this exception, the inherited `JCODE_*` environment would
/// silently create a headless hwatu page.
fn should_default_to_normal(user_initiated: bool, from_agent: bool) -> bool {
    user_initiated || !from_agent
}

/// GIO launch markers are inherited by child processes. The companion PID is
/// therefore required to prove that GIO launched this process, rather than an
/// ancestor such as a terminal or agent UI.
fn is_current_gio_launch(
    desktop_file_present: bool,
    launched_pid: Option<&str>,
    current_pid: u32,
) -> bool {
    desktop_file_present
        && launched_pid.and_then(|pid| pid.parse::<u32>().ok()) == Some(current_pid)
}

/// Parse a user-facing mode name. `focus` is accepted as an alias for
/// `normal` to match the `--focus` flag.
fn parse_open_mode(value: &str) -> Option<OpenMode> {
    match value.trim() {
        "normal" | "focus" => Some(OpenMode::Normal),
        "background" => Some(OpenMode::Background),
        "headless" => Some(OpenMode::Headless),
        _ => None,
    }
}

/// `agent_mode` from `~/.config/hwatu/config.json` (the same file the
/// daemon uses for adblock state). Absent/invalid values fall through
/// to the built-in headless default.
fn config_agent_mode() -> Option<OpenMode> {
    let base = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| std::path::PathBuf::from(h).join(".config"))
        })?;
    let raw = std::fs::read_to_string(base.join("hwatu").join("config.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&raw).ok()?;
    json.get("agent_mode")
        .and_then(|v| v.as_str())
        .and_then(parse_open_mode)
}

fn parse_with_default_mode(args: &[String], default_mode: OpenMode) -> Result<Request, String> {
    // Flags (`--app-id`, `--id`, `--timeout-ms`, `--no-wait`, `--wait`) may
    // appear anywhere relative to the subcommand/URL.
    let mut app_id: Option<String> = None;
    let mut id: Option<u64> = None;
    let mut timeout_ms: Option<u64> = None;
    let mut no_wait = false;
    let mut wait = false;
    let mut full = false;
    let mut nth: Option<u32> = None;
    let mut contains: Option<String> = None;
    let mut to_y: Option<f64> = None;
    let mut by_pages: Option<f64> = None;
    let mut r#ref: Option<u32> = None;
    let mut clear = false;
    let mut no_clear = false;
    let mut enter = false;
    let mut limit: Option<usize> = None;
    let mut time_ms: Option<f64> = None;
    let mut progress: Option<f64> = None;
    let mut resume = false;
    let mut observe = false;
    let mut observe_ms: Option<u64> = None;
    let mut other: Option<u64> = None;
    let mut baseline: Option<String> = None;
    let mut base: Option<String> = None;
    let mut use_stdin = false;
    let mut tolerance: Option<u8> = None;
    let mut heatmap: Option<String> = None;
    let mut viewports: Vec<Viewport> = Vec::new();
    let mut baseline_dir: Option<String> = None;
    let mut expect_text: Option<String> = None;
    let mut absent = false;
    let mut visible = false;
    let mut until: Option<LoadStage> = None;
    let mut eval_js: Option<String> = None;
    let mut shot = false;
    let mut shot_path: Option<String> = None;
    let mut keep = false;
    let mut diff = false;
    let mut rect = false;
    let mut budget: Option<usize> = None;
    let mut reason: Option<String> = None;
    let mut now = false;
    let mut profile: Option<String> = None;
    let mut expect_watch = false;
    let mut mode = default_mode;
    let mut trusted = false;
    let mut rest: Vec<&String> = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == "--app-id" {
            app_id = Some(
                it.next()
                    .filter(|v| !v.trim().is_empty())
                    .ok_or("usage: hwatu --app-id <id> [url]")?
                    .clone(),
            );
        } else if let Some(v) = arg.strip_prefix("--app-id=") {
            if v.trim().is_empty() {
                return Err("usage: hwatu --app-id=<id> [url]".into());
            }
            app_id = Some(v.to_string());
        } else if arg == "--id" {
            id = Some(
                it.next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("usage: --id <window-id>")?,
            );
        } else if let Some(v) = arg.strip_prefix("--id=") {
            id = Some(v.parse().map_err(|_| "usage: --id=<window-id>")?);
        } else if arg == "--timeout-ms" {
            timeout_ms = Some(
                it.next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("usage: --timeout-ms <ms>")?,
            );
        } else if let Some(v) = arg.strip_prefix("--timeout-ms=") {
            timeout_ms = Some(v.parse().map_err(|_| "usage: --timeout-ms=<ms>")?);
        } else if arg == "--no-wait" {
            no_wait = true;
        } else if arg == "--wait" {
            wait = true;
        } else if arg == "--trusted" {
            trusted = true;
        } else if arg == "--full" {
            full = true;
        } else if arg == "--nth" {
            nth = Some(
                it.next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("usage: --nth <index>")?,
            );
        } else if arg == "--contains" {
            contains = Some(
                it.next()
                    .filter(|v| !v.is_empty())
                    .ok_or("usage: --contains <text>")?
                    .clone(),
            );
        } else if arg == "--ref" {
            r#ref = Some(
                it.next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("usage: --ref <n> (from `hwatu snapshot`)")?,
            );
        } else if arg == "--clear" {
            clear = true;
        } else if arg == "--no-clear" {
            no_clear = true;
        } else if arg == "--enter" || arg == "--submit" {
            enter = true;
        } else if arg == "--limit" {
            limit = Some(
                it.next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("usage: --limit <n>")?,
            );
        } else if arg == "--budget" {
            budget = Some(
                it.next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("usage: --budget <chars>")?,
            );
        } else if let Some(v) = arg.strip_prefix("--budget=") {
            budget = Some(v.parse().map_err(|_| "usage: --budget=<chars>")?);
        } else if arg == "--reason" {
            reason = Some(
                it.next()
                    .filter(|v| !v.trim().is_empty())
                    .ok_or("usage: --reason <text>")?
                    .clone(),
            );
        } else if let Some(v) = arg.strip_prefix("--reason=") {
            if v.trim().is_empty() {
                return Err("usage: --reason=<text>".into());
            }
            reason = Some(v.to_string());
        } else if arg == "--now" {
            now = true;
        } else if arg == "--profile" {
            profile = Some(
                it.next()
                    .filter(|v| !v.trim().is_empty())
                    .ok_or("usage: --profile <name|auto>")?
                    .clone(),
            );
        } else if let Some(v) = arg.strip_prefix("--profile=") {
            if v.trim().is_empty() {
                return Err("usage: --profile=<name|auto>".into());
            }
            profile = Some(v.to_string());
        } else if arg == "--time-ms" {
            time_ms = Some(
                it.next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("usage: --time-ms <ms>")?,
            );
        } else if arg == "--progress" {
            progress = Some(
                it.next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("usage: --progress <0..1>")?,
            );
        } else if arg == "--resume" {
            resume = true;
        } else if arg == "--observe" {
            observe = true;
        } else if arg == "--ms" {
            observe_ms = Some(
                it.next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("usage: --ms <milliseconds>")?,
            );
        } else if let Some(v) = arg.strip_prefix("--ms=") {
            observe_ms = Some(v.parse().map_err(|_| "usage: --ms=<milliseconds>")?);
        } else if arg == "--other" {
            other = Some(
                it.next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("usage: --other <window-id>")?,
            );
        } else if arg == "--baseline" {
            baseline = Some(
                it.next()
                    .filter(|v| !v.is_empty())
                    .ok_or("usage: --baseline <png-path>")?
                    .clone(),
            );
        } else if arg == "--base" {
            base = Some(
                it.next()
                    .filter(|v| !v.is_empty())
                    .ok_or("usage: --base <url>")?
                    .clone(),
            );
        } else if let Some(v) = arg.strip_prefix("--base=") {
            if v.trim().is_empty() {
                return Err("usage: --base=<url>".into());
            }
            base = Some(v.to_string());
        } else if arg == "--stdin" {
            use_stdin = true;
        } else if arg == "--tolerance" {
            tolerance = Some(
                it.next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("usage: --tolerance <0-255>")?,
            );
        } else if arg == "--heatmap" {
            heatmap = Some(
                it.next()
                    .filter(|v| !v.is_empty())
                    .ok_or("usage: --heatmap <png-path>")?
                    .clone(),
            );
        } else if arg == "--viewports" {
            let list = it
                .next()
                .filter(|v| !v.trim().is_empty())
                .ok_or("usage: --viewports <WxH>[,<WxH>...] (e.g. 360x640,1920x1080)")?;
            viewports = Viewport::parse_list(list)?;
        } else if let Some(v) = arg.strip_prefix("--viewports=") {
            if v.trim().is_empty() {
                return Err("usage: --viewports=<WxH>[,<WxH>...] (e.g. 360x640,1920x1080)".into());
            }
            viewports = Viewport::parse_list(v)?;
        } else if arg == "--baseline-dir" {
            baseline_dir = Some(
                it.next()
                    .filter(|v| !v.is_empty())
                    .ok_or("usage: --baseline-dir <dir>")?
                    .clone(),
            );
        } else if let Some(v) = arg.strip_prefix("--baseline-dir=") {
            if v.trim().is_empty() {
                return Err("usage: --baseline-dir=<dir>".into());
            }
            baseline_dir = Some(v.to_string());
        } else if arg == "--text" {
            expect_text = Some(
                it.next()
                    .filter(|v| !v.is_empty())
                    .ok_or("usage: --text <substring>")?
                    .clone(),
            );
        } else if arg == "--absent" {
            absent = true;
        } else if arg == "--visible" {
            visible = true;
        } else if arg == "--watch" {
            expect_watch = true;
        } else if arg == "--until" {
            let v = it.next().ok_or("usage: --until (committed|dom|settled)")?;
            until = Some(LoadStage::parse(v).ok_or("usage: --until (committed|dom|settled)")?);
        } else if let Some(v) = arg.strip_prefix("--until=") {
            until = Some(LoadStage::parse(v).ok_or("usage: --until=(committed|dom|settled)")?);
        } else if arg == "--eval" {
            eval_js = Some(
                it.next()
                    .filter(|v| !v.trim().is_empty())
                    .ok_or("usage: --eval <js>")?
                    .clone(),
            );
        } else if arg == "--shot" {
            shot = true;
        } else if let Some(v) = arg.strip_prefix("--shot=") {
            if v.trim().is_empty() {
                return Err("usage: --shot=<png-path>".into());
            }
            shot = true;
            shot_path = Some(v.to_string());
        } else if arg == "--keep" {
            keep = true;
        } else if arg == "--diff" {
            diff = true;
        } else if arg == "--rect" {
            rect = true;
        } else if arg == "--to-y" {
            to_y = Some(
                it.next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("usage: --to-y <pixels>")?,
            );
        } else if arg == "--by" {
            by_pages = Some(
                it.next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("usage: --by <pages>")?,
            );
        } else if arg == "--background" {
            mode = OpenMode::Background;
        } else if arg == "--headless" {
            mode = OpenMode::Headless;
        } else if arg == "--focus" {
            mode = OpenMode::Normal;
        } else if arg.starts_with("--") {
            // An unknown flag must not fall through to the URL/search
            // path: `hwatu --version` used to open a web search for
            // "--version" in a visible window. Free-text tails (eval
            // JS, typed text) may legitimately contain `--tokens`, so
            // only those subcommands keep collecting them.
            let free_text_tail = matches!(
                rest.first().map(|s| s.as_str()),
                Some("eval") | Some("type")
            );
            if free_text_tail {
                rest.push(arg);
            } else if arg == "--help" {
                // `--help` anywhere outside a free-text tail prints usage
                // instead of "unknown flag"; the `-h`/`--help` subcommand
                // arm below only catches it as a bare first word.
                return Err(USAGE.to_string());
            } else {
                return Err(format!("unknown flag {arg:?}\n{USAGE}"));
            }
        } else {
            rest.push(arg);
        }
    }

    match rest.first().map(|s| s.as_str()) {
        None => Ok(Request::Open {
            url: None,
            app_id,
            mode,
            profile: resolve_profile(profile),
        }),
        Some("list") => Ok(Request::List),
        Some("ping") => Ok(Request::Ping),
        Some("quit") => Ok(Request::Quit),
        Some("close") => {
            let id = rest
                .get(1)
                .and_then(|s| s.parse().ok())
                .ok_or("usage: hwatu close <id>")?;
            Ok(Request::Close { id })
        }
        Some("eval") => {
            // Join the remaining args so unquoted JS still works.
            let js = rest[1..]
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            if js.trim().is_empty() {
                return Err("usage: hwatu eval [--id <id>] [--timeout-ms <ms>] <js>".into());
            }
            Ok(Request::Eval { id, js, timeout_ms })
        }
        Some("goto") => {
            let url = rest
                .get(1)
                .ok_or("usage: hwatu goto [--id <id>] [--no-wait] [--until <stage>] <url>")?
                .to_string();
            Ok(Request::Navigate {
                id,
                url,
                wait: !no_wait,
                until: until.unwrap_or_default(),
                timeout_ms,
            })
        }
        Some("shot") | Some("screenshot") => Ok(Request::Screenshot {
            id,
            path: rest.get(1).map(|s| s.to_string()),
            full,
        }),
        Some("wait-load") => Ok(Request::WaitLoad {
            id,
            until: until.unwrap_or_default(),
            timeout_ms,
        }),
        Some("batch") => {
            const USAGE_BATCH: &str = "usage: hwatu batch (--stdin | '<json-array>' | '{\"cmd\":\"batch\",...}')";
            let payload = if use_stdin {
                if rest.len() > 1 {
                    return Err(format!("batch takes --stdin or inline JSON, not both\n{USAGE_BATCH}"));
                }
                let mut payload = String::new();
                std::io::Read::read_to_string(&mut std::io::stdin().lock(), &mut payload)
                    .map_err(|e| format!("batch: cannot read stdin: {e}"))?;
                payload
            } else {
                let payload = rest[1..]
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                if payload.trim().is_empty() {
                    return Err(USAGE_BATCH.into());
                }
                payload
            };
            let value: serde_json::Value = serde_json::from_str(&payload)
                .map_err(|e| format!("batch: invalid JSON: {e}"))?;
            let actions = if value.is_array() {
                serde_json::from_value::<Vec<Request>>(value)
                    .map_err(|e| format!("batch: invalid action array: {e}"))?
            } else {
                let request = serde_json::from_value::<Request>(value)
                    .map_err(|e| format!("batch: invalid request: {e}"))?;
                let Request::Batch { actions } = request else {
                    return Err("batch: JSON object must be a batch request; pass an array for actions".into());
                };
                actions
            };
            Request::validate_batch(&actions).map_err(|e| format!("batch: {e}"))?;
            Ok(Request::Batch { actions })
        }
        Some("check") => {
            let url = rest
                .get(1)
                .ok_or(
                    "usage: hwatu check <url> [--eval <js>] [--shot | --shot=<png>] [--full] \
                     [--baseline <png> [--tolerance <0-255>] [--heatmap <png>]] \
                     [--viewports <WxH>[,<WxH>...] [--baseline-dir <dir>]] \
                     [--until (committed|dom|settled)] [--keep] [--timeout-ms <ms>]",
                )?
                .to_string();
            if baseline_dir.is_some() && viewports.is_empty() {
                return Err("--baseline-dir needs --viewports".into());
            }
            if baseline_dir.is_some() && baseline.is_some() {
                return Err(
                    "--baseline and --baseline-dir are mutually exclusive (per-size \
                     baselines live in the dir as <WxH>.png)"
                        .into(),
                );
            }
            Ok(Request::Check {
                url: Some(url),
                render: None,
                base: None,
                eval: eval_js,
                shot,
                shot_path,
                full,
                baseline,
                tolerance,
                heatmap,
                until: until.unwrap_or_default(),
                keep,
                timeout_ms,
                viewports,
                baseline_dir,
            })
        }
        Some("render") => {
            const USAGE_RENDER: &str = "usage: hwatu render (--stdin | <file.html>) \
                 [--base <url>] [--eval <js>] [--shot | --shot=<png>] [--full] \
                 [--baseline <png> [--tolerance <0-255>] [--heatmap <png>]] \
                 [--viewports <WxH>[,<WxH>...] [--baseline-dir <dir>]] \
                 [--until (committed|dom|settled)] [--keep] [--timeout-ms <ms>]";
            if baseline_dir.is_some() && viewports.is_empty() {
                return Err("--baseline-dir needs --viewports".into());
            }
            if baseline_dir.is_some() && baseline.is_some() {
                return Err(
                    "--baseline and --baseline-dir are mutually exclusive (per-size \
                     baselines live in the dir as <WxH>.png)"
                        .into(),
                );
            }
            let html = match (use_stdin, rest.get(1)) {
                (true, Some(_)) => {
                    return Err(format!("render takes --stdin or a file, not both\n{USAGE_RENDER}"))
                }
                (true, None) => {
                    let mut html = String::new();
                    std::io::Read::read_to_string(&mut std::io::stdin().lock(), &mut html)
                        .map_err(|e| format!("render: cannot read stdin: {e}"))?;
                    html
                }
                (false, Some(path)) => std::fs::read_to_string(path)
                    .map_err(|e| format!("render: cannot read {path}: {e}"))?,
                (false, None) => return Err(USAGE_RENDER.into()),
            };
            if html.trim().is_empty() {
                return Err("render: the document is empty".into());
            }
            if html.len() > hwatu_ipc::RENDER_MAX_BYTES {
                return Err(format!(
                    "render: document is {} bytes; the cap is {} (serve it over http \
                     and use `hwatu check` instead)",
                    html.len(),
                    hwatu_ipc::RENDER_MAX_BYTES
                ));
            }
            Ok(Request::Check {
                url: None,
                render: Some(html),
                base,
                eval: eval_js,
                shot,
                shot_path,
                full,
                baseline,
                tolerance,
                heatmap,
                until: until.unwrap_or_default(),
                keep,
                timeout_ms,
                viewports,
                baseline_dir,
            })
        }
        Some("prefetch") => {
            let url = rest
                .get(1)
                .ok_or("usage: hwatu prefetch <url>")?
                .to_string();
            Ok(Request::Prefetch { url })
        }
        Some("challenge") | Some("detect-challenge") => Ok(Request::Challenge {
            id,
            wait,
            timeout_ms,
        }),
        Some("scroll") => {
            // hwatu scroll [--id <id>] [<selector> [nth]] | --to-y <px> | --by <pages>
            // Flags --nth/--contains/--to-y/--by were consumed above.
            Ok(Request::Scroll {
                id,
                selector: rest.get(1).map(|s| s.to_string()),
                nth: rest.get(2).and_then(|s| s.parse().ok()).or(nth),
                contains,
                to_y,
                by_pages,
                timeout_ms,
            })
        }
        Some("upload") => {
            let selector = rest
                .get(1)
                .ok_or("usage: hwatu upload [--id <id>] <selector> <path>")?
                .to_string();
            let path = rest
                .get(2)
                .ok_or("usage: hwatu upload [--id <id>] <selector> <path>")?
                .to_string();
            Ok(Request::Upload {
                id,
                selector,
                path,
                timeout_ms,
            })
        }
        Some("snapshot") => Ok(Request::Snapshot {
            id,
            diff,
            rect,
            budget,
            timeout_ms,
        }),
        Some("expect") => {
            let selector = rest
                .get(1)
                .ok_or(
                    "usage: hwatu expect [--id <id>] <selector> [--contains <filter>] \
                     [--text <substring>] [--absent] [--visible] [--nth <n>] [--timeout-ms <ms>]",
                )?
                .to_string();
            Ok(Request::Expect {
                id,
                selector,
                nth,
                contains,
                text: expect_text,
                absent,
                visible,
                timeout_ms,
                watch: expect_watch,
            })
        }
        Some("motion") => Ok(Request::Motion {
            id,
            observe,
            observe_ms,
            timeout_ms,
        }),
        Some("resize") => {
            let usage = "usage: hwatu resize [--id <id>] <width>x<height>";
            let size = rest.get(1).ok_or(usage)?;
            let (w, h) = size.split_once(['x', 'X']).ok_or(usage)?;
            Ok(Request::Resize {
                id,
                width: w.trim().parse().map_err(|_| usage)?,
                height: h.trim().parse().map_err(|_| usage)?,
            })
        }
        Some("seek") => Ok(Request::Seek {
            id,
            time_ms,
            progress,
            resume,
            timeout_ms,
        }),
        Some("clock") => {
            const USAGE_CLOCK: &str = "usage: hwatu clock [--id <id>] (pause | resume | step <ms> | set <ms> | seed <u64> | status)";
            let (action, ms, seed) = match rest.get(1).map(|s| s.as_str()) {
                Some("pause") => (ClockAction::Pause, None, None),
                Some("resume") => (ClockAction::Resume, None, None),
                None | Some("status") => (ClockAction::Status, None, None),
                Some("step") => (
                    ClockAction::Step,
                    Some(rest.get(2).and_then(|s| s.parse().ok()).ok_or(USAGE_CLOCK)?),
                    None,
                ),
                Some("set") => (
                    ClockAction::Set,
                    Some(rest.get(2).and_then(|s| s.parse().ok()).ok_or(USAGE_CLOCK)?),
                    None,
                ),
                Some("seed") => (
                    ClockAction::Seed,
                    None,
                    Some(rest.get(2).and_then(|s| s.parse().ok()).ok_or(USAGE_CLOCK)?),
                ),
                Some(other) => {
                    return Err(format!("unknown clock action {other:?}\n{USAGE_CLOCK}"))
                }
            };
            Ok(Request::Clock {
                id,
                action,
                ms,
                seed,
                timeout_ms,
            })
        }
        Some("diff") => Ok(Request::Diff {
            id: id.ok_or("usage: hwatu diff --id <id> (--other <id> | --baseline <png>) [--tolerance <n>] [--heatmap <png>] [--full]")?,
            other,
            baseline,
            tolerance,
            heatmap,
            full,
            timeout_ms,
        }),
        Some("click") => {
            let selector = rest.get(1).map(|s| s.to_string());
            if selector.is_none() && r#ref.is_none() {
                return Err(
                    "usage: hwatu click [--id <id>] [--trusted] <selector> [--nth <n>] [--contains <text>] \
                     | --ref <n>"
                        .into(),
                );
            }
            Ok(Request::Click {
                id,
                selector,
                nth: rest.get(2).and_then(|s| s.parse().ok()).or(nth),
                contains,
                r#ref,
                trusted,
                timeout_ms,
            })
        }
        Some("type") => {
            // hwatu type <selector> <text...> | --ref <n> <text...>
            let (selector, text_args) = if r#ref.is_some() {
                (None, &rest[1..])
            } else {
                (
                    Some(
                        rest.get(1)
                            .ok_or(
                                "usage: hwatu type [--id <id>] [--trusted] (<selector> | --ref <n>) <text> \
                                 [--enter] [--no-clear]",
                            )?
                            .to_string(),
                    ),
                    &rest[2.min(rest.len())..],
                )
            };
            let text = text_args
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            if text.is_empty() {
                return Err(
                    "usage: hwatu type [--id <id>] [--trusted] (<selector> | --ref <n>) <text> [--enter] \
                     [--no-clear]"
                        .into(),
                );
            }
            Ok(Request::Type {
                id,
                selector,
                nth,
                contains,
                r#ref,
                text,
                trusted,
                clear: !no_clear,
                enter,
                timeout_ms,
            })
        }
        Some("press") => {
            const USAGE_PRESS: &str = "usage: hwatu press [--id <id>] (Tab | Enter)";
            if rest.len() != 2 {
                return Err(USAGE_PRESS.into());
            }
            let key = PressKey::parse(rest[1]).ok_or(USAGE_PRESS)?;
            Ok(Request::Press {
                id,
                key,
                timeout_ms,
            })
        }
        Some("paste") => {
            let selector = if r#ref.is_some() {
                None
            } else {
                Some(
                    rest.get(1)
                        .ok_or("usage: hwatu paste [--id <id>] (<selector> | --ref <n>)")?
                        .to_string(),
                )
            };
            if selector.is_none() && r#ref.is_none() {
                return Err("usage: hwatu paste [--id <id>] (<selector> | --ref <n>)".into());
            }
            Ok(Request::Paste {
                id,
                selector,
                nth,
                contains,
                r#ref,
                timeout_ms,
            })
        }
        Some("console") => Ok(Request::Console { id, clear, limit }),
        Some("net") => Ok(Request::Net { id, clear, limit }),
        Some("history") => {
            // `hwatu history [query...] [--limit N] [--clear]`
            let query = rest[1..]
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            Ok(Request::History {
                query,
                limit,
                clear,
            })
        }
        Some("clear-site-data") => Ok(Request::ClearSiteData {
            host: rest.get(1).map(|s| s.to_string()),
        }),
        Some("handoff") => {
            // `hwatu handoff <id> --reason <text> [--now]`
            let win = rest
                .get(1)
                .and_then(|s| s.parse().ok())
                .or(id)
                .ok_or("usage: hwatu handoff <id> --reason <text> [--now]")?;
            let reason = reason
                .clone()
                .ok_or("usage: hwatu handoff <id> --reason <text> [--now]")?;
            Ok(Request::Handoff {
                id: win,
                reason,
                now,
            })
        }
        Some("handoffs") => Ok(Request::Handoffs {
            take: rest.get(1).and_then(|s| s.parse().ok()),
        }),
        Some("jump") => {
            let query = rest[1..]
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            if query.is_empty() {
                return Err("usage: hwatu jump <query>".into());
            }
            Ok(Request::Jump { query, open: true })
        }
        Some("focus") => {
            let id = rest
                .get(1)
                .and_then(|s| s.parse().ok())
                .or(id)
                .ok_or("usage: hwatu focus <id>")?;
            Ok(Request::Focus { id })
        }
        Some("unfocus") | Some("hide") => {
            let id = rest
                .get(1)
                .and_then(|s| s.parse().ok())
                .or(id)
                .ok_or("usage: hwatu unfocus <id>")?;
            Ok(Request::Unfocus { id })
        }
        Some("adblock") => {
            let action = match rest.get(1).map(|s| s.as_str()) {
                Some("on") => AdblockCmd::On,
                Some("off") => AdblockCmd::Off,
                None | Some("status") => AdblockCmd::Status,
                Some("update") => AdblockCmd::Update,
                Some(other) => {
                    return Err(format!(
                    "unknown adblock action {other:?}; usage: hwatu adblock [on|off|status|update]"
                ))
                }
            };
            Ok(Request::Adblock { action })
        }
        Some("-h") | Some("--help") => Err(USAGE.to_string()),
        // Everything else is a URL or search query. Join the words so
        // `hwatu how to exit vim` searches without needing quotes.
        Some(_) => Ok(Request::Open {
            url: Some(
                rest.iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            app_id,
            mode,
            profile: resolve_profile(profile),
        }),
    }
}

/// Resolve the profile choice (platform item 6): explicit flag wins,
/// then HWATU_PROFILE. The value `auto` derives a stable per-worktree
/// name from the git repo root (hash of the toplevel path), so N
/// agents in N worktrees get cookie isolation with zero flags —
/// export HWATU_PROFILE=auto once in the agent harness.
fn resolve_profile(flag: Option<String>) -> Option<String> {
    let choice = flag.or_else(|| {
        std::env::var("HWATU_PROFILE")
            .ok()
            .filter(|v| !v.is_empty())
    })?;
    if choice != "auto" {
        return Some(choice);
    }
    // auto: hash the git toplevel (or cwd outside a repo).
    let root = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
        })?;
    // FNV-1a, matching the daemon's session-file hashing style.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in root.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    Some(format!("wt-{hash:016x}"))
}

const USAGE: &str = "usage: hwatu [--app-id <id>] [--profile <name|auto>] [--background|--headless|--focus] [url] \
(agent environments default to --headless; set HWATU_AGENT_MODE or \
\"agent_mode\" in ~/.config/hwatu/config.json to normal|background|headless) \
| list [--json] | close <id> | focus <id> | unfocus <id> \
	| eval [--id <id>] [--timeout-ms <ms>] <js> | goto [--id <id>] [--no-wait] [--until <stage>] <url> \
	| batch (--stdin | '<json-action-array>' | '{\"cmd\":\"batch\",...}') \
	    | shot [--id <id>] [--full] [path] | wait-load [--id <id>] [--until (committed|dom|settled)] \
    | check <url> [--eval <js>] [--shot | --shot=<png>] [--full] [--baseline <png> [--tolerance <0-255>] [--heatmap <png>]] [--viewports <WxH>[,<WxH>...] [--baseline-dir <dir>]] [--until <stage>] [--keep] \
    | render (--stdin | <file.html>) [--base <url>] [--eval <js>] [--shot | --shot=<png>] [--full] [--baseline <png> ...] [--until <stage>] [--keep] \
    | prefetch <url> \
    | watch [--id <id>] [--kinds load,console,download,window,expect] \
    | challenge [--id <id>] [--wait] \
    | upload [--id <id>] <selector> <path> \
| scroll [--id <id>] [<selector> [nth]] [--contains <text>] [--to-y <px>] [--by <pages>] \
| snapshot [--id <id>] [--diff] [--rect] [--budget <chars>] \
| history [<query>] [--limit <n>] [--clear] \
| clear-site-data [<host>] \
| handoff <id> --reason <text> [--now] | handoffs [<id>] \
| jump <query> \
| expect [--id <id>] <selector> [--contains <filter>] [--text <substring>] [--absent] [--visible] [--nth <n>] [--timeout-ms <ms>] [--watch] \
	| click [--id <id>] [--trusted] (<selector> [nth] [--contains <text>] | --ref <n>) \
	| type [--id <id>] [--trusted] (<selector> | --ref <n>) <text> [--enter] [--no-clear] \
	| press [--id <id>] (Tab | Enter) \
	| paste [--id <id>] (<selector> | --ref <n>) \
	| console [--id <id>] [--clear] [--limit <n>] \
| net [--id <id>] [--clear] [--limit <n>] \
| motion [--id <id>] [--observe [--ms <ms>]] \
| resize [--id <id>] <width>x<height> \
| seek [--id <id>] (--time-ms <ms> | --progress <0..1> | --resume) \
| clock [--id <id>] (pause | resume | step <ms> | set <ms> | seed <u64> | status) \
| diff --id <id> (--other <id> | --baseline <png>) [--tolerance <0-255>] [--heatmap <png>] [--full] \
| clone <url> [--out <dir>] [--viewport <WxH>] [--tolerance <0-255>] [--no-verify] [--keep] \
| adblock [on|off|status|update] \
| doctor | setup [--client claude|cursor|generic|jcode] [--scope project|user] [--dry-run] [--undo] \
| demo [url] [--focus] \
| mcp | update | ping | quit";

/// `hwatu watch`: subscribe and stream events as JSON lines until the
/// daemon goes away or we are killed. The shell-agent face of push
/// IPC: `hwatu watch | while read -r event; do ...; done`.
fn watch(args: &[String]) -> i32 {
    const USAGE_WATCH: &str =
        "usage: hwatu watch [--id <window-id>] [--kinds load,console,download,window,expect]";
    let mut window: Option<u64> = None;
    let mut kinds: Option<Vec<String>> = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == "--id" {
            window = match it.next().and_then(|v| v.parse().ok()) {
                Some(v) => Some(v),
                None => {
                    eprintln!("{USAGE_WATCH}");
                    return 2;
                }
            };
        } else if arg == "--kinds" {
            let Some(list) = it.next() else {
                eprintln!("{USAGE_WATCH}");
                return 2;
            };
            let parsed: Vec<String> = list
                .split(',')
                .map(|k| k.trim().to_string())
                .filter(|k| !k.is_empty())
                .collect();
            for k in &parsed {
                if !hwatu_ipc::EVENT_KINDS.contains(&k.as_str()) {
                    eprintln!(
                        "hwatu: unknown event kind {k:?} (want one of {:?})",
                        hwatu_ipc::EVENT_KINDS
                    );
                    return 2;
                }
            }
            kinds = Some(parsed);
        } else {
            eprintln!("{USAGE_WATCH}");
            return 2;
        }
    }

    let mut stream = match connect_or_spawn() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("hwatu: cannot reach daemon: {e}");
            return 1;
        }
    };
    let request = Request::Subscribe { kinds, window };
    let mut payload = serde_json::to_vec(&request).expect("serialize request");
    payload.push(b'\n');
    if let Err(e) = stream.write_all(&payload) {
        eprintln!("hwatu: write failed: {e}");
        return 1;
    }
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        match line {
            Ok(l) if !l.trim().is_empty() => {
                // An old daemon answers with a one-shot error and EOF;
                // surface it as an error, not as an "event".
                if l.contains("\"status\":\"err\"") {
                    eprintln!("hwatu: {l}");
                    if l.contains("unknown variant") {
                        eprintln!(
                            "hwatu: the running daemon predates `watch`; restart it: \
                             hwatu quit && hwatu ping"
                        );
                    }
                    return 1;
                }
                println!("{l}");
            }
            Ok(_) => {}
            Err(_) => break, // daemon went away
        }
    }
    0
}

/// `hwatu expect ... --watch`: subscribe first so the daemon's initial
/// expect event cannot race past the client, then install the resident
/// monitor on a one-shot connection and stream only `expect` events.
fn expect_watch(request: Request) -> i32 {
    let window = match &request {
        Request::Expect { id, .. } => *id,
        _ => None,
    };
    let mut stream = match connect_or_spawn() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("hwatu: cannot reach daemon: {e}");
            return 1;
        }
    };
    let sub = Request::Subscribe {
        kinds: Some(vec!["expect".to_string()]),
        window,
    };
    let mut payload = serde_json::to_vec(&sub).expect("serialize request");
    payload.push(b'\n');
    if let Err(e) = stream.write_all(&payload) {
        eprintln!("hwatu: write failed: {e}");
        return 1;
    }

    let mut reader = BufReader::new(stream);
    let mut first = String::new();
    if let Err(e) = reader.read_line(&mut first) {
        eprintln!("hwatu: read failed: {e}");
        return 1;
    }
    if first.contains("\"status\":\"err\"") {
        eprintln!("hwatu: {first}");
        return 1;
    }
    print!("{first}");
    let _ = std::io::stdout().flush();

    let mut install = match connect_or_spawn() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("hwatu: cannot reach daemon: {e}");
            return 1;
        }
    };
    let mut payload = serde_json::to_vec(&request).expect("serialize request");
    payload.push(b'\n');
    if let Err(e) = install.write_all(&payload) {
        eprintln!("hwatu: write failed: {e}");
        return 1;
    }
    let mut install_line = String::new();
    let mut install_reader = BufReader::new(install);
    if let Err(e) = install_reader.read_line(&mut install_line) {
        eprintln!("hwatu: read failed: {e}");
        return 1;
    }
    if install_line.contains("\"status\":\"err\"") {
        eprintln!("hwatu: {install_line}");
        return 1;
    }

    for line in reader.lines() {
        match line {
            Ok(l) if !l.trim().is_empty() => {
                println!("{l}");
                if expect_event_is_terminal_navigation(&l) {
                    break;
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    0
}

fn expect_event_is_terminal_navigation(line: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        return false;
    };
    value.get("event").and_then(|v| v.as_str()) == Some("expect")
        && value.pointer("/data/phase").and_then(|v| v.as_str()) == Some("navigation")
        && value
            .pointer("/data/result/terminal")
            .and_then(|v| v.as_bool())
            == Some(true)
}

pub(crate) fn connect_or_spawn() -> std::io::Result<UnixStream> {
    let path = hwatu_ipc::socket_path();
    if let Ok(s) = UnixStream::connect(&path) {
        return Ok(s);
    }

    // No daemon: spawn hwatud (sibling binary or PATH) and poll the socket.
    let daemon = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("hwatud")))
        .filter(|p| p.exists())
        .unwrap_or_else(|| "hwatud".into());
    Command::new(daemon)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(s) = UnixStream::connect(&path) {
            return Ok(s);
        }
        if Instant::now() > deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "daemon did not come up within 10s",
            ));
        }
        std::thread::sleep(Duration::from_millis(15));
    }
}

#[cfg(test)]
mod tests {
    use super::{
        is_current_gio_launch, is_onboarding_command, normalize_request_paths,
        parse_with_default_mode, should_default_to_normal, OpenMode,
    };
    use hwatu_ipc::{PressKey, Request};

    #[test]
    fn onboarding_commands_are_handled_before_url_parsing() {
        assert!(is_onboarding_command(Some("doctor")));
        assert!(is_onboarding_command(Some("setup")));
        assert!(is_onboarding_command(Some("demo")));
        assert!(!is_onboarding_command(Some("https://example.com")));
        assert!(!is_onboarding_command(None));
    }

    #[test]
    fn desktop_entry_launch_is_visible_inside_agent_environment() {
        assert!(should_default_to_normal(true, true));
        assert!(should_default_to_normal(false, false));
        assert!(!should_default_to_normal(false, true));
    }

    #[test]
    fn gio_launch_marker_must_belong_to_current_process() {
        assert!(is_current_gio_launch(true, Some("42"), 42));
        assert!(!is_current_gio_launch(true, Some("41"), 42));
        assert!(!is_current_gio_launch(true, None, 42));
        assert!(!is_current_gio_launch(true, Some("not-a-pid"), 42));
        assert!(!is_current_gio_launch(false, Some("42"), 42));
    }

    /// Env-independent parse: tests themselves often run under a
    /// coding agent, which would flip `default_open_mode()`.
    fn parse(args: &[String]) -> Result<Request, String> {
        let mut request = parse_with_default_mode(args, OpenMode::Normal)?;
        normalize_request_paths(&mut request);
        Ok(request)
    }

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn check_parses_flags() {
        let Ok(Request::Check {
            url,
            eval,
            shot,
            shot_path,
            until,
            keep,
            ..
        }) = parse(&args(&[
            "check",
            "example.com",
            "--eval",
            "return document.title",
            "--shot=/tmp/x.png",
            "--until",
            "dom",
        ]))
        else {
            panic!("expected Check");
        };
        assert_eq!(url.as_deref(), Some("example.com"));
        assert_eq!(eval.as_deref(), Some("return document.title"));
        assert!(shot);
        assert_eq!(shot_path.as_deref(), Some("/tmp/x.png"));
        assert_eq!(until, hwatu_ipc::LoadStage::Dom);
        assert!(!keep);
    }

    #[test]
    fn daemon_paths_are_resolved_against_client_cwd() {
        let cwd = std::env::current_dir().unwrap();
        let expected = |path: &str| cwd.join(path).to_string_lossy().into_owned();

        let Ok(Request::Check {
            shot_path,
            baseline,
            heatmap,
            ..
        }) = parse(&args(&[
            "check",
            "example.com",
            "--shot=artifacts/after.png",
            "--baseline",
            "baselines/before.png",
            "--heatmap",
            "artifacts/diff.png",
        ]))
        else {
            panic!("expected Check");
        };
        assert_eq!(
            shot_path.as_deref(),
            Some(expected("artifacts/after.png").as_str())
        );
        assert_eq!(
            baseline.as_deref(),
            Some(expected("baselines/before.png").as_str())
        );
        assert_eq!(
            heatmap.as_deref(),
            Some(expected("artifacts/diff.png").as_str())
        );

        let Ok(Request::Check { baseline_dir, .. }) = parse(&args(&[
            "check",
            "example.com",
            "--viewports",
            "360x640",
            "--baseline-dir",
            "baselines",
        ])) else {
            panic!("expected viewport Check");
        };
        assert_eq!(
            baseline_dir.as_deref(),
            Some(expected("baselines").as_str())
        );

        let Ok(Request::Screenshot { path, .. }) = parse(&args(&["shot", "shot.png"])) else {
            panic!("expected Screenshot");
        };
        assert_eq!(path.as_deref(), Some(expected("shot.png").as_str()));

        let Ok(Request::Upload { path, .. }) = parse(&args(&["upload", "input", "file.txt"]))
        else {
            panic!("expected Upload");
        };
        assert_eq!(path, expected("file.txt"));

        let Ok(Request::Diff {
            baseline, heatmap, ..
        }) = parse(&args(&[
            "diff",
            "--id",
            "1",
            "--baseline",
            "before.png",
            "--heatmap",
            "diff.png",
        ]))
        else {
            panic!("expected Diff");
        };
        assert_eq!(baseline.as_deref(), Some(expected("before.png").as_str()));
        assert_eq!(heatmap.as_deref(), Some(expected("diff.png").as_str()));
    }

    #[test]
    fn check_parses_baseline_diff_flags() {
        let Ok(Request::Check {
            baseline,
            tolerance,
            heatmap,
            ..
        }) = parse(&args(&[
            "check",
            "localhost:3000",
            "--baseline",
            "/tmp/base.png",
            "--tolerance",
            "12",
            "--heatmap",
            "/tmp/heat.png",
        ]))
        else {
            panic!("expected Check");
        };
        assert_eq!(baseline.as_deref(), Some("/tmp/base.png"));
        assert_eq!(tolerance, Some(12));
        assert_eq!(heatmap.as_deref(), Some("/tmp/heat.png"));
    }

    /// `hwatu render <file>` reads the file into a render-check; the
    /// composing flags (--base/--eval/--shot/--until/--keep) ride
    /// along, and `url` stays empty. --stdin plus a file is a usage
    /// error, as is neither, an empty document, and --base without a
    /// value.
    #[test]
    fn render_parses_file_and_flags() {
        let dir = std::env::temp_dir().join(format!("hwatu-render-parse-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("page.html");
        std::fs::write(&file, "<h1>hello</h1>").unwrap();
        let file = file.to_string_lossy().to_string();

        let Ok(Request::Check {
            url,
            render,
            base,
            eval,
            shot,
            until,
            keep,
            ..
        }) = parse(&args(&[
            "render",
            &file,
            "--base",
            "http://localhost:3000/",
            "--eval",
            "document.title",
            "--shot",
            "--until",
            "dom",
            "--keep",
        ]))
        else {
            panic!("expected Check");
        };
        assert_eq!(url, None);
        assert_eq!(render.as_deref(), Some("<h1>hello</h1>"));
        assert_eq!(base.as_deref(), Some("http://localhost:3000/"));
        assert_eq!(eval.as_deref(), Some("document.title"));
        assert!(shot);
        assert_eq!(until, hwatu_ipc::LoadStage::Dom);
        assert!(keep);

        // Conflicts and gaps are usage errors.
        assert!(parse(&args(&["render"])).is_err());
        assert!(parse(&args(&["render", "--stdin", &file])).is_err());
        assert!(parse(&args(&["render", &file, "--base"])).is_err());
        assert!(parse(&args(&["render", "/nonexistent/x.html"])).is_err());
        std::fs::write(dir.join("empty.html"), "  \n").unwrap();
        assert!(parse(&args(&[
            "render",
            &dir.join("empty.html").to_string_lossy()
        ]))
        .is_err());

        // Oversized documents are rejected client-side with the cap named.
        let big = dir.join("big.html");
        std::fs::write(&big, "x".repeat(hwatu_ipc::RENDER_MAX_BYTES + 1)).unwrap();
        let err = parse(&args(&["render", &big.to_string_lossy()])).unwrap_err();
        assert!(err.contains("cap"), "got: {err}");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn prefetch_parses_and_requires_url() {
        let Ok(Request::Prefetch { url }) = parse(&args(&["prefetch", "localhost:3000"])) else {
            panic!("expected Prefetch");
        };
        assert_eq!(url, "localhost:3000");
        assert!(parse(&args(&["prefetch"])).is_err());
    }

    #[test]
    fn expect_parses_watch_assertion() {
        let Ok(Request::Expect {
            id,
            selector,
            nth,
            contains,
            text,
            absent,
            visible,
            timeout_ms,
            watch,
        }) = parse(&args(&[
            "expect",
            "--id",
            "3",
            "#status",
            "--contains",
            "phase",
            "--text",
            "ready",
            "--visible",
            "--nth",
            "2",
            "--timeout-ms",
            "1234",
            "--watch",
        ]))
        else {
            panic!("expected Expect");
        };
        assert_eq!(id, Some(3));
        assert_eq!(selector, "#status");
        assert_eq!(nth, Some(2));
        assert_eq!(contains.as_deref(), Some("phase"));
        assert_eq!(text.as_deref(), Some("ready"));
        assert!(!absent);
        assert!(visible);
        assert_eq!(timeout_ms, Some(1234));
        assert!(watch);
    }

    #[test]
    fn expect_watch_terminal_navigation_detection_is_specific() {
        let terminal =
            r#"{"event":"expect","data":{"phase":"navigation","result":{"terminal":true}}}"#;
        assert!(super::expect_event_is_terminal_navigation(terminal));
        let flip = r#"{"event":"expect","data":{"phase":"flip","result":{"terminal":false}}}"#;
        assert!(!super::expect_event_is_terminal_navigation(flip));
        let load = r#"{"event":"load","data":{"phase":"navigation","result":{"terminal":true}}}"#;
        assert!(!super::expect_event_is_terminal_navigation(load));
    }

    #[test]
    fn check_defaults_are_minimal() {
        let Ok(Request::Check {
            eval,
            shot,
            shot_path,
            baseline,
            until,
            keep,
            ..
        }) = parse(&args(&["check", "example.com"]))
        else {
            panic!("expected Check");
        };
        assert_eq!(eval, None);
        assert!(!shot);
        assert_eq!(shot_path, None);
        assert_eq!(baseline, None);
        assert_eq!(until, hwatu_ipc::LoadStage::Settled);
        assert!(!keep);
    }

    /// `--viewports` parses a size list into the sweep field; bad
    /// sizes, empty lists, and flag conflicts are usage errors; a
    /// plain check keeps an empty sweep (old behavior).
    #[test]
    fn check_parses_viewports() {
        let Ok(Request::Check {
            viewports,
            baseline_dir,
            ..
        }) = parse(&args(&[
            "check",
            "localhost:3000",
            "--viewports",
            "360x640,768x1024,1920x1080",
            "--baseline-dir",
            "/tmp/base",
        ]))
        else {
            panic!("expected Check");
        };
        assert_eq!(
            viewports,
            vec![
                hwatu_ipc::Viewport { w: 360, h: 640 },
                hwatu_ipc::Viewport { w: 768, h: 1024 },
                hwatu_ipc::Viewport { w: 1920, h: 1080 },
            ]
        );
        assert_eq!(baseline_dir.as_deref(), Some("/tmp/base"));

        // `--viewports=` form works too.
        let Ok(Request::Check { viewports, .. }) =
            parse(&args(&["check", "x.test", "--viewports=800x600"]))
        else {
            panic!("expected Check");
        };
        assert_eq!(viewports, vec![hwatu_ipc::Viewport { w: 800, h: 600 }]);

        // Invalid sizes are rejected at parse time, naming the entry.
        for bad in ["banana", "360", "360x", "0x640", "-1x640", "99999x2", ""] {
            assert!(
                parse(&args(&["check", "x.test", "--viewports", bad])).is_err(),
                "size {bad:?} should be rejected"
            );
        }
        // Flag coherence: baseline-dir needs viewports; --baseline
        // conflicts with a sweep's per-size baselines.
        assert!(parse(&args(&["check", "x.test", "--baseline-dir", "/tmp/b"])).is_err());
        assert!(parse(&args(&[
            "check",
            "x.test",
            "--viewports",
            "360x640",
            "--baseline",
            "/tmp/b.png",
            "--baseline-dir",
            "/tmp/b",
        ]))
        .is_err());

        // No sweep flags: the request carries an empty sweep.
        let Ok(Request::Check {
            viewports,
            baseline_dir,
            ..
        }) = parse(&args(&["check", "x.test"]))
        else {
            panic!("expected Check");
        };
        assert!(viewports.is_empty());
        assert_eq!(baseline_dir, None);
    }

    /// `hwatu render --viewports ...` sweeps rendered markup too.
    #[test]
    fn render_parses_viewports() {
        let dir = std::env::temp_dir().join(format!("hwatu-vp-parse-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("page.html");
        std::fs::write(&file, "<h1>vp</h1>").unwrap();
        let file = file.to_string_lossy().to_string();

        let Ok(Request::Check {
            render, viewports, ..
        }) = parse(&args(&["render", &file, "--viewports", "360x640,1024x768"]))
        else {
            panic!("expected Check");
        };
        assert_eq!(render.as_deref(), Some("<h1>vp</h1>"));
        assert_eq!(viewports.len(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn until_applies_to_goto_and_wait_load() {
        let Ok(Request::Navigate { until, .. }) =
            parse(&args(&["goto", "--until", "committed", "example.com"]))
        else {
            panic!("expected Navigate");
        };
        assert_eq!(until, hwatu_ipc::LoadStage::Committed);
        let Ok(Request::WaitLoad { until, .. }) = parse(&args(&["wait-load", "--until=dom"]))
        else {
            panic!("expected WaitLoad");
        };
        assert_eq!(until, hwatu_ipc::LoadStage::Dom);
        // Bare wait-load keeps full-settle semantics.
        let Ok(Request::WaitLoad { until, .. }) = parse(&args(&["wait-load"])) else {
            panic!("expected WaitLoad");
        };
        assert_eq!(until, hwatu_ipc::LoadStage::Settled);
        assert!(parse(&args(&["wait-load", "--until", "nonsense"])).is_err());
    }

    #[test]
    fn agent_default_applies_and_focus_overrides() {
        let Ok(Request::Open { mode, .. }) =
            parse_with_default_mode(&args(&["example.com"]), OpenMode::Headless)
        else {
            panic!("expected Open");
        };
        assert_eq!(mode, OpenMode::Headless);
        let Ok(Request::Open { mode, .. }) =
            parse_with_default_mode(&args(&["--focus", "example.com"]), OpenMode::Headless)
        else {
            panic!("expected Open");
        };
        assert_eq!(mode, OpenMode::Normal);
    }

    #[test]
    fn mode_names_parse() {
        use super::parse_open_mode;
        assert_eq!(parse_open_mode("normal"), Some(OpenMode::Normal));
        assert_eq!(parse_open_mode("focus"), Some(OpenMode::Normal));
        assert_eq!(parse_open_mode("background"), Some(OpenMode::Background));
        assert_eq!(parse_open_mode(" headless "), Some(OpenMode::Headless));
        assert_eq!(parse_open_mode("visible"), None);
        assert_eq!(parse_open_mode(""), None);
    }

    /// `hwatu --version` (or any typo'd flag) must error out, not open
    /// a visible window searching the web for the flag text.
    #[test]
    fn unknown_flag_is_rejected_not_searched() {
        let err = parse(&args(&["--version"])).unwrap_err();
        assert!(err.contains("unknown flag"), "got: {err}");
        let err = parse(&args(&["--headles", "example.com"])).unwrap_err();
        assert!(err.contains("unknown flag"), "got: {err}");
    }

    /// Eval JS and typed text may legitimately contain `--tokens`.
    #[test]
    fn eval_and_type_keep_double_dash_tokens() {
        let Ok(Request::Eval { js, .. }) = parse(&args(&["eval", "return", "'--version'"])) else {
            panic!("expected Eval");
        };
        assert!(js.contains("--version"));
        let Ok(Request::Type { text, .. }) =
            parse(&args(&["type", "input", "--verbose", "output"]))
        else {
            panic!("expected Type");
        };
        assert_eq!(text, "--verbose output");
    }

    #[test]
    fn bare_open_has_no_app_id() {
        assert!(matches!(
            parse(&args(&[])),
            Ok(Request::Open {
                url: None,
                app_id: None,
                mode: OpenMode::Normal,
                profile: None,
            })
        ));
    }

    #[test]
    fn background_flag_sets_mode() {
        let Ok(Request::Open { url, mode, .. }) = parse(&args(&["--background", "localhost:3000"]))
        else {
            panic!("expected Open");
        };
        assert_eq!(url.as_deref(), Some("localhost:3000"));
        assert_eq!(mode, OpenMode::Background);
    }

    #[test]
    fn headless_flag_sets_mode_any_position() {
        let Ok(Request::Open { url, mode, .. }) = parse(&args(&["example.com", "--headless"]))
        else {
            panic!("expected Open");
        };
        assert_eq!(url.as_deref(), Some("example.com"));
        assert_eq!(mode, OpenMode::Headless);
    }

    /// `hwatu clock seed <u64>` parses to the Seed action; a missing
    /// or non-numeric seed is a usage error, and plain `clock status`
    /// carries no seed.
    #[test]
    fn clock_seed_parses() {
        use hwatu_ipc::ClockAction;
        let Ok(Request::Clock {
            action, ms, seed, ..
        }) = parse(&args(&["clock", "seed", "42"]))
        else {
            panic!("expected Clock");
        };
        assert_eq!(action, ClockAction::Seed);
        assert_eq!(ms, None);
        assert_eq!(seed, Some(42));
        assert!(parse(&args(&["clock", "seed"])).is_err());
        assert!(parse(&args(&["clock", "seed", "nope"])).is_err());
        let Ok(Request::Clock { action, seed, .. }) = parse(&args(&["clock", "status"])) else {
            panic!("expected Clock");
        };
        assert_eq!(action, ClockAction::Status);
        assert_eq!(seed, None);
    }

    #[test]
    fn multiword_query_joins_into_one_open() {
        let Ok(Request::Open { url, .. }) = parse(&args(&["how", "to", "exit", "vim"])) else {
            panic!("expected Open");
        };
        assert_eq!(url.as_deref(), Some("how to exit vim"));
    }

    #[test]
    fn app_id_before_url() {
        let Ok(Request::Open { url, app_id, .. }) =
            parse(&args(&["--app-id", "mail", "gmail.com"]))
        else {
            panic!("expected Open");
        };
        assert_eq!(url.as_deref(), Some("gmail.com"));
        assert_eq!(app_id.as_deref(), Some("mail"));
    }

    #[test]
    fn app_id_after_url_and_equals_form() {
        let Ok(Request::Open { url, app_id, .. }) = parse(&args(&["gmail.com", "--app-id=mail"]))
        else {
            panic!("expected Open");
        };
        assert_eq!(url.as_deref(), Some("gmail.com"));
        assert_eq!(app_id.as_deref(), Some("mail"));
    }

    #[test]
    fn app_id_without_url_opens_home() {
        let Ok(Request::Open { url, app_id, .. }) = parse(&args(&["--app-id", "scratch"])) else {
            panic!("expected Open");
        };
        assert!(url.is_none());
        assert_eq!(app_id.as_deref(), Some("scratch"));
    }

    #[test]
    fn app_id_missing_value_errors() {
        assert!(parse(&args(&["--app-id"])).is_err());
        assert!(parse(&args(&["--app-id="])).is_err());
    }

    #[test]
    fn subcommands_still_parse() {
        assert!(matches!(parse(&args(&["list"])), Ok(Request::List)));
        assert!(matches!(parse(&args(&["ping"])), Ok(Request::Ping)));
        assert!(matches!(
            parse(&args(&["close", "3"])),
            Ok(Request::Close { id: 3 })
        ));
    }

    #[test]
    fn challenge_parses_detect_and_manual_wait() {
        assert!(matches!(
            parse(&args(&["challenge"])),
            Ok(Request::Challenge {
                id: None,
                wait: false,
                timeout_ms: None
            })
        ));
        assert!(matches!(
            parse(&args(&[
                "detect-challenge",
                "--id",
                "7",
                "--wait",
                "--timeout-ms",
                "2500"
            ])),
            Ok(Request::Challenge {
                id: Some(7),
                wait: true,
                timeout_ms: Some(2500)
            })
        ));
    }

    #[test]
    fn shot_full_flag() {
        let Ok(Request::Screenshot { full, path, .. }) =
            parse(&args(&["shot", "--full", "/tmp/x.png"]))
        else {
            panic!("expected Screenshot");
        };
        assert!(full);
        assert_eq!(path.as_deref(), Some("/tmp/x.png"));
        let Ok(Request::Screenshot { full, .. }) = parse(&args(&["shot"])) else {
            panic!("expected Screenshot");
        };
        assert!(!full);
    }

    #[test]
    fn scroll_selector_with_nth_and_contains() {
        let Ok(Request::Scroll {
            selector,
            nth,
            contains,
            to_y,
            by_pages,
            ..
        }) = parse(&args(&["scroll", "h3", "2", "--contains", "Joule"]))
        else {
            panic!("expected Scroll");
        };
        assert_eq!(selector.as_deref(), Some("h3"));
        assert_eq!(nth, Some(2));
        assert_eq!(contains.as_deref(), Some("Joule"));
        assert!(to_y.is_none() && by_pages.is_none());
    }

    #[test]
    fn scroll_by_pages_and_to_y() {
        let Ok(Request::Scroll {
            selector, by_pages, ..
        }) = parse(&args(&["scroll", "--by", "-0.5"]))
        else {
            panic!("expected Scroll");
        };
        assert!(selector.is_none());
        assert_eq!(by_pages, Some(-0.5));
        let Ok(Request::Scroll { to_y, .. }) = parse(&args(&["scroll", "--to-y", "1200"])) else {
            panic!("expected Scroll");
        };
        assert_eq!(to_y, Some(1200.0));
    }

    #[test]
    fn bare_scroll_defaults_to_one_page() {
        let Ok(Request::Scroll {
            selector,
            to_y,
            by_pages,
            ..
        }) = parse(&args(&["scroll"]))
        else {
            panic!("expected Scroll");
        };
        // All None: the daemon treats this as by_pages = 1.0.
        assert!(selector.is_none() && to_y.is_none() && by_pages.is_none());
    }

    #[test]
    fn snapshot_parses() {
        assert!(matches!(
            parse(&args(&["snapshot"])),
            Ok(Request::Snapshot {
                id: None,
                diff: false,
                rect: false,
                ..
            })
        ));
        assert!(matches!(
            parse(&args(&["snapshot", "--id", "3"])),
            Ok(Request::Snapshot { id: Some(3), .. })
        ));
        assert!(matches!(
            parse(&args(&["snapshot", "--diff"])),
            Ok(Request::Snapshot {
                id: None,
                diff: true,
                rect: false,
                ..
            })
        ));
        assert!(matches!(
            parse(&args(&["snapshot", "--diff", "--id", "3"])),
            Ok(Request::Snapshot {
                id: Some(3),
                diff: true,
                rect: false,
                ..
            })
        ));
        assert!(matches!(
            parse(&args(&["snapshot", "--rect"])),
            Ok(Request::Snapshot {
                rect: true,
                diff: false,
                ..
            })
        ));
    }

    #[test]
    fn click_selector_with_disambiguation() {
        let Ok(Request::Click {
            selector,
            nth,
            contains,
            r#ref,
            ..
        }) = parse(&args(&["click", "a", "2", "--contains", "Docs"]))
        else {
            panic!("expected Click");
        };
        assert_eq!(selector.as_deref(), Some("a"));
        assert_eq!(nth, Some(2));
        assert_eq!(contains.as_deref(), Some("Docs"));
        assert!(r#ref.is_none());
    }

    #[test]
    fn click_by_ref_and_bare_click_errors() {
        let Ok(Request::Click {
            selector,
            r#ref,
            trusted,
            ..
        }) = parse(&args(&["click", "--trusted", "--ref", "7"]))
        else {
            panic!("expected Click");
        };
        assert!(selector.is_none());
        assert_eq!(r#ref, Some(7));
        assert!(trusted);
        assert!(parse(&args(&["click"])).is_err());
    }

    #[test]
    fn type_joins_text_and_flags() {
        let Ok(Request::Type {
            selector,
            text,
            trusted,
            clear,
            enter,
            ..
        }) = parse(&args(&[
            "type",
            "input[name=q]",
            "rust",
            "borrow",
            "checker",
            "--trusted",
            "--enter",
        ]))
        else {
            panic!("expected Type");
        };
        assert_eq!(selector.as_deref(), Some("input[name=q]"));
        assert_eq!(text, "rust borrow checker");
        assert!(trusted);
        assert!(clear);
        assert!(enter);
    }

    #[test]
    fn type_by_ref_no_clear_and_missing_text_errors() {
        let Ok(Request::Type {
            selector,
            r#ref,
            text,
            clear,
            ..
        }) = parse(&args(&["type", "--ref", "4", "--no-clear", "hello"]))
        else {
            panic!("expected Type");
        };
        assert!(selector.is_none());
        assert_eq!(r#ref, Some(4));
        assert_eq!(text, "hello");
        assert!(!clear);
        assert!(parse(&args(&["type", "input"])).is_err());
        assert!(parse(&args(&["type"])).is_err());
    }

    #[test]
    fn press_parses_tab_enter_and_optional_id() {
        assert!(matches!(
            parse(&args(&["press", "Tab"])),
            Ok(Request::Press {
                id: None,
                key: PressKey::Tab,
                timeout_ms: None,
            })
        ));
        assert!(matches!(
            parse(&args(&[
                "press",
                "--id",
                "7",
                "--timeout-ms",
                "500",
                "enter"
            ])),
            Ok(Request::Press {
                id: Some(7),
                key: PressKey::Enter,
                timeout_ms: Some(500),
            })
        ));
    }

    #[test]
    fn press_rejects_missing_unknown_and_extra_keys() {
        assert!(parse(&args(&["press"])).is_err());
        assert!(parse(&args(&["press", "Escape"])).is_err());
        assert!(parse(&args(&["press", "Tab", "Enter"])).is_err());
    }

    #[test]
    fn paste_targets_selector_or_ref() {
        let Ok(Request::Paste {
            selector,
            nth,
            contains,
            r#ref,
            ..
        }) = parse(&args(&[
            "paste",
            "textarea",
            "--nth",
            "2",
            "--contains",
            "Bio",
        ]))
        else {
            panic!("expected Paste");
        };
        assert_eq!(selector.as_deref(), Some("textarea"));
        assert_eq!(nth, Some(2));
        assert_eq!(contains.as_deref(), Some("Bio"));
        assert!(r#ref.is_none());

        let Ok(Request::Paste {
            selector, r#ref, ..
        }) = parse(&args(&["paste", "--ref", "4"]))
        else {
            panic!("expected Paste");
        };
        assert!(selector.is_none());
        assert_eq!(r#ref, Some(4));
        assert!(parse(&args(&["paste"])).is_err());
    }

    #[test]
    fn console_flags() {
        assert!(matches!(
            parse(&args(&["console"])),
            Ok(Request::Console {
                clear: false,
                limit: None,
                ..
            })
        ));
        assert!(matches!(
            parse(&args(&["console", "--clear", "--limit", "20"])),
            Ok(Request::Console {
                clear: true,
                limit: Some(20),
                ..
            })
        ));
    }

    #[test]
    fn net_flags() {
        assert!(matches!(
            parse(&args(&["net"])),
            Ok(Request::Net {
                id: None,
                clear: false,
                limit: None,
            })
        ));
        assert!(matches!(
            parse(&args(&["net", "--id", "3", "--clear", "--limit", "20"])),
            Ok(Request::Net {
                id: Some(3),
                clear: true,
                limit: Some(20),
            })
        ));
    }
}
