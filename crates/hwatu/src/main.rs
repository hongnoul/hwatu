//! hana: thin client for the hwatud browser daemon.
//!
//! `hana <url>` opens a window in ~1 IPC roundtrip. If no daemon is
//! running, it spawns one and waits for the socket.

mod mcp;
mod update;

use hwatu_ipc::{AdblockCmd, ClockAction, OpenMode, Request, Response};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::Command;
use std::time::{Duration, Instant};

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("update") {
        std::process::exit(update::run());
    }
    if args.first().map(String::as_str) == Some("mcp") {
        std::process::exit(mcp::run());
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

fn parse(args: &[String]) -> Result<Request, String> {
    parse_with_default_mode(args, default_open_mode())
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
    // JCODE_-prefixed var as an agent marker — except user-config knobs that
    // people export session-wide from .profile/environment.d (those would
    // make the whole desktop look like an agent and force every WM-keybind
    // launch headless).
    const JCODE_USER_CONFIG_VARS: &[&str] = &["JCODE_NO_AUTO_UPDATE", "JCODE_BING_API_KEY"];
    let from_jcode = std::env::vars_os().any(|(k, _)| {
        k.to_str()
            .map(|k| k.starts_with("JCODE_") && !JCODE_USER_CONFIG_VARS.contains(&k))
            .unwrap_or(false)
    });
    if !from_jcode && !AGENT_MARKERS.iter().any(|k| std::env::var_os(k).is_some()) {
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
    let mut other: Option<u64> = None;
    let mut baseline: Option<String> = None;
    let mut tolerance: Option<u8> = None;
    let mut heatmap: Option<String> = None;
    let mut expect_text: Option<String> = None;
    let mut absent = false;
    let mut mode = default_mode;
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
        } else if arg == "--text" {
            expect_text = Some(
                it.next()
                    .filter(|v| !v.is_empty())
                    .ok_or("usage: --text <substring>")?
                    .clone(),
            );
        } else if arg == "--absent" {
            absent = true;
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
                .ok_or("usage: hwatu goto [--id <id>] [--no-wait] <url>")?
                .to_string();
            Ok(Request::Navigate {
                id,
                url,
                wait: !no_wait,
                timeout_ms,
            })
        }
        Some("shot") | Some("screenshot") => Ok(Request::Screenshot {
            id,
            path: rest.get(1).map(|s| s.to_string()),
            full,
        }),
        Some("wait-load") => Ok(Request::WaitLoad { id, timeout_ms }),
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
        Some("snapshot") => Ok(Request::Snapshot { id, timeout_ms }),
        Some("expect") => {
            let selector = rest
                .get(1)
                .ok_or(
                    "usage: hwatu expect [--id <id>] <selector> [--contains <filter>] \
                     [--text <substring>] [--absent] [--nth <n>] [--timeout-ms <ms>]",
                )?
                .to_string();
            Ok(Request::Expect {
                id,
                selector,
                nth,
                contains,
                text: expect_text,
                absent,
                timeout_ms,
            })
        }
        Some("motion") => Ok(Request::Motion { id, timeout_ms }),
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
            const USAGE_CLOCK: &str =
                "usage: hwatu clock [--id <id>] (pause | resume | step <ms> | set <ms> | status)";
            let (action, ms) = match rest.get(1).map(|s| s.as_str()) {
                Some("pause") => (ClockAction::Pause, None),
                Some("resume") => (ClockAction::Resume, None),
                None | Some("status") => (ClockAction::Status, None),
                Some("step") => (
                    ClockAction::Step,
                    Some(rest.get(2).and_then(|s| s.parse().ok()).ok_or(USAGE_CLOCK)?),
                ),
                Some("set") => (
                    ClockAction::Set,
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
                    "usage: hwatu click [--id <id>] <selector> [--nth <n>] [--contains <text>] \
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
                                "usage: hwatu type [--id <id>] (<selector> | --ref <n>) <text> \
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
                    "usage: hwatu type [--id <id>] (<selector> | --ref <n>) <text> [--enter] \
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
                clear: !no_clear,
                enter,
                timeout_ms,
            })
        }
        Some("console") => Ok(Request::Console { id, clear, limit }),
        Some("focus") => {
            let id = rest
                .get(1)
                .and_then(|s| s.parse().ok())
                .or(id)
                .ok_or("usage: hwatu focus <id>")?;
            Ok(Request::Focus { id })
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
        }),
    }
}

const USAGE: &str = "usage: hwatu [--app-id <id>] [--background|--headless|--focus] [url] \
(agent environments default to --headless; set HWATU_AGENT_MODE or \
\"agent_mode\" in ~/.config/hwatu/config.json to normal|background|headless) \
| list [--json] | close <id> | focus <id> \
| eval [--id <id>] [--timeout-ms <ms>] <js> | goto [--id <id>] [--no-wait] <url> \
    | shot [--id <id>] [--full] [path] | wait-load [--id <id>] | challenge [--id <id>] [--wait] \
    | upload [--id <id>] <selector> <path> \
| scroll [--id <id>] [<selector> [nth]] [--contains <text>] [--to-y <px>] [--by <pages>] \
| snapshot [--id <id>] \
| expect [--id <id>] <selector> [--contains <filter>] [--text <substring>] [--absent] [--nth <n>] [--timeout-ms <ms>] \
| click [--id <id>] (<selector> [nth] [--contains <text>] | --ref <n>) \
| type [--id <id>] (<selector> | --ref <n>) <text> [--enter] [--no-clear] \
| console [--id <id>] [--clear] [--limit <n>] \
| motion [--id <id>] \
| resize [--id <id>] <width>x<height> \
| seek [--id <id>] (--time-ms <ms> | --progress <0..1> | --resume) \
| clock [--id <id>] (pause | resume | step <ms> | set <ms> | status) \
| diff --id <id> (--other <id> | --baseline <png>) [--tolerance <0-255>] [--heatmap <png>] [--full] \
| adblock [on|off|status|update] | mcp | update | ping | quit";

fn connect_or_spawn() -> std::io::Result<UnixStream> {
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
    use super::{parse_with_default_mode, OpenMode};
    use hwatu_ipc::Request;

    /// Env-independent parse: tests themselves often run under a
    /// coding agent, which would flip `default_open_mode()`.
    fn parse(args: &[String]) -> Result<Request, String> {
        parse_with_default_mode(args, OpenMode::Normal)
    }

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
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
            Ok(Request::Snapshot { id: None, .. })
        ));
        assert!(matches!(
            parse(&args(&["snapshot", "--id", "3"])),
            Ok(Request::Snapshot { id: Some(3), .. })
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
            selector, r#ref, ..
        }) = parse(&args(&["click", "--ref", "7"]))
        else {
            panic!("expected Click");
        };
        assert!(selector.is_none());
        assert_eq!(r#ref, Some(7));
        assert!(parse(&args(&["click"])).is_err());
    }

    #[test]
    fn type_joins_text_and_flags() {
        let Ok(Request::Type {
            selector,
            text,
            clear,
            enter,
            ..
        }) = parse(&args(&[
            "type",
            "input[name=q]",
            "rust",
            "borrow",
            "checker",
            "--enter",
        ]))
        else {
            panic!("expected Type");
        };
        assert_eq!(selector.as_deref(), Some("input[name=q]"));
        assert_eq!(text, "rust borrow checker");
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
}
