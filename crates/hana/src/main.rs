//! hana: thin client for the hanad browser daemon.
//!
//! `hana <url>` opens a window in ~1 IPC roundtrip. If no daemon is
//! running, it spawns one and waits for the socket.

use hana_ipc::{Request, Response};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::Command;
use std::time::{Duration, Instant};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
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
            eprintln!("hana: cannot reach daemon: {e}");
            std::process::exit(1);
        }
    };

    let mut payload = serde_json::to_vec(&request).expect("serialize request");
    payload.push(b'\n');
    if let Err(e) = stream.write_all(&payload) {
        eprintln!("hana: write failed: {e}");
        std::process::exit(1);
    }

    let mut line = String::new();
    let mut reader = BufReader::new(stream);
    if let Err(e) = reader.read_line(&mut line) {
        eprintln!("hana: read failed: {e}");
        std::process::exit(1);
    }

    match serde_json::from_str::<Response>(line.trim()) {
        Ok(Response::Ok { window, windows }) => {
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
                    println!("{}\t{}\t{}", w.id, w.url, w.title);
                }
            }
        }
        Ok(Response::Err { message }) => {
            eprintln!("hana: {message}");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("hana: bad response: {e} ({line:?})");
            std::process::exit(1);
        }
    }
}

fn parse(args: &[String]) -> Result<Request, String> {
    match args.first().map(String::as_str) {
        None => Ok(Request::Open { url: None, app_id: None }),
        Some("list") => Ok(Request::List),
        Some("ping") => Ok(Request::Ping),
        Some("quit") => Ok(Request::Quit),
        Some("close") => {
            let id = args
                .get(1)
                .and_then(|s| s.parse().ok())
                .ok_or("usage: hana close <id>")?;
            Ok(Request::Close { id })
        }
        Some("-h") | Some("--help") => Err(USAGE.to_string()),
        Some(url) => Ok(Request::Open {
            url: Some(url.to_string()),
            app_id: None,
        }),
    }
}

const USAGE: &str = "usage: hana [url | list | close <id> | ping | quit]";

fn connect_or_spawn() -> std::io::Result<UnixStream> {
    let path = hana_ipc::socket_path();
    if let Ok(s) = UnixStream::connect(&path) {
        return Ok(s);
    }

    // No daemon: spawn hanad (sibling binary or PATH) and poll the socket.
    let daemon = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("hanad")))
        .filter(|p| p.exists())
        .unwrap_or_else(|| "hanad".into());
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
