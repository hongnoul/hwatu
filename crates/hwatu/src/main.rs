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
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("update") {
        std::process::exit(update::run());
    }
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
        }) => {
            if let Some(w) = window {
                println!(
                    "window {} -> {} ({} ms)",
                    w.id,
                    w.url,
                    started.elapsed().as_millis()
                );
            }
            if let Some(ws) = windows {
                for w in ws {
                    let flag = if w.suspended { "suspended" } else { "live" };
                    println!("{}\t{}\t{}\t{}", w.id, flag, w.url, w.title);
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
    match args.first().map(String::as_str) {
        None => Ok(Request::Open {
            url: None,
            app_id: None,
        }),
        Some("list") => Ok(Request::List),
        Some("ping") => Ok(Request::Ping),
        Some("quit") => Ok(Request::Quit),
        Some("close") => {
            let id = args
                .get(1)
                .and_then(|s| s.parse().ok())
                .ok_or("usage: hwatu close <id>")?;
            Ok(Request::Close { id })
        }
        Some("adblock") => {
            let action = match args.get(1).map(String::as_str) {
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
            app_id: None,
        }),
    }
}

const USAGE: &str =
    "usage: hwatu [url | list | close <id> | adblock [on|off|status|update] | update | ping | quit]";

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
