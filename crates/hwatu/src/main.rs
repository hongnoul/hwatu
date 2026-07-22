//! hana: thin client for the hwatud browser daemon.
//!
//! `hana <url>` opens a window in ~1 IPC roundtrip. If no daemon is
//! running, it spawns one and waits for the socket.

mod update;

use hwatu_ipc::{AdblockCmd, OpenMode, Request, Response};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::Command;
use std::time::{Duration, Instant};

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("update") {
        std::process::exit(update::run());
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
/// `Background` so verification flows never steal the user's focus,
/// while human entries keep Normal. Explicit flags always win, and
/// `--focus` opts an agent back into Normal deliberately.
fn default_open_mode() -> OpenMode {
    const AGENT_MARKERS: &[&str] = &[
        "JCODE_SOCKET",  // jcode
        "CLAUDECODE",    // Claude Code
        "CODEX_SANDBOX", // Codex CLI
        "CURSOR_AGENT",  // Cursor CLI
        "AGENT",         // Amp, and a de-facto generic marker
        "OPENCODE",      // opencode
        "GEMINI_CLI",    // Gemini CLI
    ];
    if AGENT_MARKERS.iter().any(|k| std::env::var_os(k).is_some()) {
        OpenMode::Background
    } else {
        OpenMode::Normal
    }
}

fn parse_with_default_mode(args: &[String], default_mode: OpenMode) -> Result<Request, String> {
    // Flags (`--app-id`, `--id`, `--timeout-ms`, `--no-wait`) may
    // appear anywhere relative to the subcommand/URL.
    let mut app_id: Option<String> = None;
    let mut id: Option<u64> = None;
    let mut timeout_ms: Option<u64> = None;
    let mut no_wait = false;
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
(agent environments default to --background) \
| list [--json] | close <id> | focus <id> \
| eval [--id <id>] [--timeout-ms <ms>] <js> | goto [--id <id>] [--no-wait] <url> \
| shot [--id <id>] [--full] [path] | wait-load [--id <id>] | upload [--id <id>] <selector> <path> \
| scroll [--id <id>] [<selector> [nth]] [--contains <text>] [--to-y <px>] [--by <pages>] \
| snapshot [--id <id>] \
| click [--id <id>] (<selector> [nth] [--contains <text>] | --ref <n>) \
| type [--id <id>] (<selector> | --ref <n>) <text> [--enter] [--no-clear] \
| console [--id <id>] [--clear] [--limit <n>] \
| adblock [on|off|status|update] | update | ping | quit";

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
    fn agent_default_is_background_and_focus_overrides() {
        let Ok(Request::Open { mode, .. }) =
            parse_with_default_mode(&args(&["example.com"]), OpenMode::Background)
        else {
            panic!("expected Open");
        };
        assert_eq!(mode, OpenMode::Background);
        let Ok(Request::Open { mode, .. }) =
            parse_with_default_mode(&args(&["--focus", "example.com"]), OpenMode::Background)
        else {
            panic!("expected Open");
        };
        assert_eq!(mode, OpenMode::Normal);
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
        }) = parse(&args(&["type", "input[name=q]", "rust", "borrow", "checker", "--enter"]))
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
