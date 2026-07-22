//! Unix-socket IPC server, integrated with the GLib main loop so all
//! window work happens on the GTK main thread.

use crate::{adblock::Adblock, automation, window::BrowserWindow, Daemon};
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
            let reply: automation::Reply = Box::new(move |response: Response| {
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
            });
            match serde_json::from_str::<Request>(line.trim()) {
                Ok(req) => dispatch(&daemon, req, reply),
                Err(e) => reply(Response::err(format!("bad request: {e}"))),
            }
        },
    );
}

/// Route one request. Most commands answer synchronously; the
/// automation commands (eval/navigate/screenshot/wait_load) complete
/// later on the main loop and consume `reply` when they finish.
fn dispatch(daemon: &Rc<Daemon>, req: Request, reply: automation::Reply) {
    // Async paths hand the reply off and return.
    match req {
        Request::Eval { id, js, timeout_ms } => {
            return automation::eval(daemon, id, js, timeout_ms, reply);
        }
        Request::Navigate {
            id,
            url,
            wait,
            timeout_ms,
        } => {
            return automation::navigate(daemon, id, url, wait, timeout_ms, reply);
        }
        Request::Screenshot { id, path, full } => {
            return automation::screenshot(daemon, id, path, full, reply);
        }
        Request::WaitLoad { id, timeout_ms } => {
            return automation::wait_load(daemon, id, timeout_ms, reply);
        }
        Request::Challenge {
            id,
            wait,
            timeout_ms,
        } => {
            return automation::challenge(daemon, id, wait, timeout_ms, reply);
        }
        Request::Upload {
            id,
            selector,
            path,
            timeout_ms,
        } => {
            return automation::upload(daemon, id, selector, path, timeout_ms, reply);
        }
        Request::Scroll {
            id,
            selector,
            nth,
            contains,
            to_y,
            by_pages,
            timeout_ms,
        } => {
            return automation::scroll(
                daemon, id, selector, nth, contains, to_y, by_pages, timeout_ms, reply,
            );
        }
        Request::Snapshot { id, timeout_ms } => {
            return automation::snapshot(daemon, id, timeout_ms, reply);
        }
        Request::Expect {
            id,
            selector,
            nth,
            contains,
            text,
            absent,
            timeout_ms,
        } => {
            return automation::expect(
                daemon, id, selector, nth, contains, text, absent, timeout_ms, reply,
            );
        }
        Request::Click {
            id,
            selector,
            nth,
            contains,
            r#ref,
            timeout_ms,
        } => {
            return automation::click(
                daemon, id, selector, nth, contains, r#ref, timeout_ms, reply,
            );
        }
        Request::Type {
            id,
            selector,
            nth,
            contains,
            r#ref,
            text,
            clear,
            enter,
            timeout_ms,
        } => {
            return automation::type_text(
                daemon, id, selector, nth, contains, r#ref, text, clear, enter, timeout_ms, reply,
            );
        }
        Request::Motion { id, timeout_ms } => {
            return crate::verify::motion(daemon, id, timeout_ms, reply);
        }
        Request::Seek {
            id,
            time_ms,
            progress,
            resume,
            timeout_ms,
        } => {
            return crate::verify::seek(daemon, id, time_ms, progress, resume, timeout_ms, reply);
        }
        Request::Diff {
            id,
            other,
            baseline,
            tolerance,
            heatmap,
            full,
            timeout_ms: _,
        } => {
            return crate::verify::diff(
                daemon, id, other, baseline, tolerance, heatmap, full, reply,
            );
        }
        _ => {}
    }

    let response = match req {
        // Ping doubles as the version handshake: the daemon reports
        // the git commit and crate version it was built from, so the
        // CLI (and agents) can detect a stale running daemon after an
        // upgrade instead of hitting "unknown variant" errors blind.
        Request::Ping => Response::value(serde_json::json!({
            "build": env!("HWATU_GIT_HASH"),
            "version": env!("CARGO_PKG_VERSION"),
        })),
        Request::Console { id, clear, limit } => automation::console(daemon, id, clear, limit),
        Request::Open { url, app_id, mode } => {
            let url = url.map(normalize_url);
            let info = BrowserWindow::open(daemon, url, app_id, mode);
            // A fresh open is the natural target for follow-up id-less
            // automation ("open, then eval").
            daemon.last_target.replace(Some(info.id));
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
        Request::Focus { id } => {
            let win = daemon.windows.borrow().get(&id).cloned();
            match win {
                Some(w) => {
                    // Focus promotes any window to normal: an agent (or
                    // user) explicitly asking for a background/headless
                    // window means they want to see it now.
                    w.present();
                    daemon.last_target.replace(Some(id));
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
        // Handled above; unreachable but keeps the match exhaustive.
        Request::Eval { .. }
        | Request::Navigate { .. }
        | Request::Screenshot { .. }
        | Request::WaitLoad { .. }
        | Request::Challenge { .. }
        | Request::Upload { .. }
        | Request::Scroll { .. }
        | Request::Snapshot { .. }
        | Request::Click { .. }
        | Request::Type { .. }
        | Request::Motion { .. }
        | Request::Seek { .. }
        | Request::Diff { .. }
        | Request::Expect { .. } => Response::err("internal: async request in sync path"),
    };
    reply(response);
}

/// Turn bar/CLI input into a loadable URL: explicit schemes and
/// `about:` pass through, bare hosts get `https://` (`http://` for
/// loopback: `localhost`, `*.localhost`, `127.*`, `[::1]`, since local
/// dev servers rarely speak TLS), and anything that doesn't look like
/// a URL becomes a web search with the configured engine (see
/// [`crate::search`]). Shared with the in-window URL bar so both
/// entry points resolve input identically.
pub fn normalize_url(input: String) -> String {
    let input = input.trim().to_string();
    if input.contains("://") || input.starts_with("about:") {
        input
    } else if is_loopback_host(&input) {
        format!("http://{input}")
    } else if looks_like_host(&input) {
        format!("https://{input}")
    } else {
        crate::search::url_for(&input)
    }
}

/// Heuristic for scheme-less input: URL, not search query? A single
/// whitespace-free token whose host part contains a dot.
fn looks_like_host(input: &str) -> bool {
    if input.is_empty() || input.contains(char::is_whitespace) {
        return false;
    }
    let host = input.split(['/', '?', '#']).next().unwrap_or(input);
    host.contains('.') && !host.starts_with('.') && !host.ends_with('.')
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

    #[test]
    fn queries_become_searches() {
        // The engine is user-configured, so assert against the search
        // module rather than a hardcoded engine URL.
        assert_eq!(
            normalize_url("rust borrow checker".into()),
            crate::search::url_for("rust borrow checker")
        );
        assert_eq!(normalize_url("vim".into()), crate::search::url_for("vim"));
        assert_eq!(
            normalize_url("  what is 2+2?  ".into()),
            crate::search::url_for("what is 2+2?")
        );
        // Any whitespace means search, even with a dot: real URLs
        // never contain raw spaces.
        assert_eq!(
            normalize_url("example.com login page".into()),
            crate::search::url_for("example.com login page")
        );
        // Dotted single tokens stay URLs, path and all.
        assert_eq!(
            normalize_url("example.com/a?b=1".into()),
            "https://example.com/a?b=1"
        );
        // Trailing/leading dots are not hosts.
        assert_eq!(
            normalize_url("what.".into()),
            crate::search::url_for("what.")
        );
    }
}
