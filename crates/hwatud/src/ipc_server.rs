//! Unix-socket IPC server, integrated with the GLib main loop so all
//! window work happens on the GTK main thread.

use crate::{adblock::Adblock, window::BrowserWindow, Daemon};
use gtk::gio;
use gtk::gio::prelude::*;
use gtk::glib;
use hwatu_ipc::{AdblockCmd, Request, Response};
use std::rc::Rc;

pub fn start(daemon: Rc<Daemon>) -> std::io::Result<()> {
    let path = hwatu_ipc::socket_path();
    // Stale socket from a dead daemon: remove and rebind.
    let _ = std::fs::remove_file(&path);

    let listener = gio::SocketListener::new();
    let addr = gio::UnixSocketAddress::new(&path);
    listener
        .add_address(
            &addr,
            gio::SocketType::Stream,
            gio::SocketProtocol::Default,
            glib::Object::NONE,
        )
        .map_err(|e| std::io::Error::other(e.to_string()))?;

    accept_next(listener, daemon);
    Ok(())
}

fn accept_next(listener: gio::SocketListener, daemon: Rc<Daemon>) {
    listener
        .clone()
        .accept_async(gio::Cancellable::NONE, move |res| {
            if let Ok((conn, _)) = res {
                handle_conn(conn, daemon.clone());
            }
            accept_next(listener, daemon);
        });
}

fn handle_conn(conn: gio::SocketConnection, daemon: Rc<Daemon>) {
    let input = gio::DataInputStream::new(&conn.input_stream());
    input.clone().read_line_async(
        glib::Priority::DEFAULT,
        gio::Cancellable::NONE,
        move |res| {
            let line = match res {
                Ok(l) => String::from_utf8_lossy(&l).into_owned(),
                Err(_) => return,
            };
            let response = match serde_json::from_str::<Request>(line.trim()) {
                Ok(req) => dispatch(&daemon, req),
                Err(e) => Response::err(format!("bad request: {e}")),
            };
            let mut out = serde_json::to_vec(&response).unwrap_or_default();
            out.push(b'\n');
            let stream = conn.output_stream();
            stream.write_all_async(
                out,
                glib::Priority::DEFAULT,
                gio::Cancellable::NONE,
                move |_| {
                    // conn dropped here; client sees EOF after response.
                    let _ = conn.close(gio::Cancellable::NONE);
                },
            );
        },
    );
}

fn dispatch(daemon: &Rc<Daemon>, req: Request) -> Response {
    match req {
        Request::Ping => Response::ok(),
        Request::Open { url, app_id } => {
            let url = url.map(normalize_url);
            let info = BrowserWindow::open(daemon, url, app_id);
            Response::window(info)
        }
        Request::List => {
            let windows = daemon.windows.borrow();
            let mut infos: Vec<_> = windows.values().map(|w| w.info()).collect();
            infos.sort_by_key(|w| w.id);
            Response::windows(infos)
        }
        Request::Close { id } => {
            let win = daemon.windows.borrow_mut().remove(&id);
            match win {
                Some(w) => {
                    w.close();
                    Response::ok()
                }
                None => Response::err(format!("no window {id}")),
            }
        }
        Request::Adblock { action } => {
            match action {
                AdblockCmd::On => Adblock::set_enabled(daemon, true),
                AdblockCmd::Off => Adblock::set_enabled(daemon, false),
                AdblockCmd::Update => Adblock::update(daemon),
                AdblockCmd::Status => {}
            }
            Response::adblock(daemon.adblock.status())
        }
        Request::Quit => {
            // Reply first, then exit from an idle callback so the response
            // actually reaches the client before the process dies.
            glib::timeout_add_local_once(std::time::Duration::from_millis(50), || {
                // Clean quit: do not resurrect these windows next start.
                crate::session::clear();
                let _ = std::fs::remove_file(hwatu_ipc::socket_path());
                std::process::exit(0);
            });
            Response::ok()
        }
    }
}

/// `example.com` -> `https://example.com`; keep schemes and about: intact.
/// Loopback hosts (`localhost`, `*.localhost`, `127.*`, `[::1]`) get `http://`
/// since local dev servers rarely speak TLS. Shared with the in-window
/// URL bar so both entry points resolve input identically.
pub fn normalize_url(input: String) -> String {
    if input.contains("://") || input.starts_with("about:") {
        input
    } else if is_loopback_host(&input) {
        format!("http://{input}")
    } else {
        format!("https://{input}")
    }
}

/// True if the host part of a scheme-less input is a loopback address.
fn is_loopback_host(input: &str) -> bool {
    let rest = input.split(['/', '?', '#']).next().unwrap_or(input);
    if rest.starts_with("[::1]") {
        return true;
    }
    let host = rest.split(':').next().unwrap_or(rest);
    host.eq_ignore_ascii_case("localhost")
        || host.to_ascii_lowercase().ends_with(".localhost")
        || host.starts_with("127.")
}

#[cfg(test)]
mod tests {
    use super::normalize_url;

    #[test]
    fn normalizes_urls() {
        assert_eq!(normalize_url("example.com".into()), "https://example.com");
        assert_eq!(
            normalize_url("localhost:3000".into()),
            "http://localhost:3000"
        );
        assert_eq!(
            normalize_url("localhost:3000/path?q=1".into()),
            "http://localhost:3000/path?q=1"
        );
        assert_eq!(
            normalize_url("127.0.0.1:8080".into()),
            "http://127.0.0.1:8080"
        );
        assert_eq!(normalize_url("[::1]:3000".into()), "http://[::1]:3000");
        assert_eq!(
            normalize_url("app.localhost:3000".into()),
            "http://app.localhost:3000"
        );
        assert_eq!(
            normalize_url("https://localhost:3000".into()),
            "https://localhost:3000"
        );
        assert_eq!(normalize_url("about:blank".into()), "about:blank");
        assert_eq!(
            normalize_url("localhost.example.com".into()),
            "https://localhost.example.com"
        );
    }
}
