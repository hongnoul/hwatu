//! Unix-socket IPC server, integrated with the GLib main loop so all
//! window work happens on the GTK main thread.

use crate::{window::BrowserWindow, Daemon};
use gtk::gio;
use gtk::gio::prelude::*;
use gtk::glib;
use hwatu_ipc::{Request, Response};
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
        Request::Quit => {
            // Reply first, then exit from an idle callback so the response
            // actually reaches the client before the process dies.
            glib::timeout_add_local_once(std::time::Duration::from_millis(50), || {
                let _ = std::fs::remove_file(hwatu_ipc::socket_path());
                std::process::exit(0);
            });
            Response::ok()
        }
    }
}

/// `example.com` -> `https://example.com`; keep schemes and about: intact.
fn normalize_url(input: String) -> String {
    if input.contains("://") || input.starts_with("about:") {
        input
    } else {
        format!("https://{input}")
    }
}
