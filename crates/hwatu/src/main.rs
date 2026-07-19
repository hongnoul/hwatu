//! hana: thin client for the hwatud browser daemon.
//!
//! `hana <url>` opens a window in ~1 IPC roundtrip. If no daemon is
//! running, it spawns one and waits for the socket.

mod update;

use hwatu_ipc::{AdblockCmd, Request, Response};
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
    // Flags (`--app-id`, `--id`, `--timeout-ms`, `--no-wait`) may
    // appear anywhere relative to the subcommand/URL.
    let mut app_id: Option<String> = None;
    let mut id: Option<u64> = None;
    let mut timeout_ms: Option<u64> = None;
    let mut no_wait = false;
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
        } else {
            rest.push(arg);
        }
    }

    match rest.first().map(|s| s.as_str()) {
        None => Ok(Request::Open { url: None, app_id }),
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
        }),
        Some("wait-load") => Ok(Request::WaitLoad { id, timeout_ms }),
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
        Some(url) => Ok(Request::Open {
            url: Some(url.to_string()),
            app_id,
        }),
    }
}

const USAGE: &str = "usage: hwatu [--app-id <id>] [url] | list [--json] | close <id> | focus <id> \
| eval [--id <id>] [--timeout-ms <ms>] <js> | goto [--id <id>] [--no-wait] <url> \
| shot [--id <id>] [path] | wait-load [--id <id>] | adblock [on|off|status|update] | update | ping | quit";

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
    use super::parse;
    use hwatu_ipc::Request;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn bare_open_has_no_app_id() {
        assert!(matches!(
            parse(&args(&[])),
            Ok(Request::Open {
                url: None,
                app_id: None
            })
        ));
    }

    #[test]
    fn app_id_before_url() {
        let Ok(Request::Open { url, app_id }) = parse(&args(&["--app-id", "mail", "gmail.com"]))
        else {
            panic!("expected Open");
        };
        assert_eq!(url.as_deref(), Some("gmail.com"));
        assert_eq!(app_id.as_deref(), Some("mail"));
    }

    #[test]
    fn app_id_after_url_and_equals_form() {
        let Ok(Request::Open { url, app_id }) = parse(&args(&["gmail.com", "--app-id=mail"]))
        else {
            panic!("expected Open");
        };
        assert_eq!(url.as_deref(), Some("gmail.com"));
        assert_eq!(app_id.as_deref(), Some("mail"));
    }

    #[test]
    fn app_id_without_url_opens_home() {
        let Ok(Request::Open { url, app_id }) = parse(&args(&["--app-id", "scratch"])) else {
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
}
