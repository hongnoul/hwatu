// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! Unix-socket IPC server, integrated with the GLib main loop so all
//! window work happens on the GTK main thread.

use crate::{adblock::Adblock, automation, window::BrowserWindow, Daemon};
use gtk::gio;
use gtk::gio::prelude::*;
use gtk::glib;
use hwatu_ipc::{AdblockCmd, BatchResult, BatchStepResult, BatchStepStatus, Request, Response};
use std::cell::RefCell;
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
    read_next_request(conn, input, daemon);
}

fn read_next_request(conn: gio::SocketConnection, input: gio::DataInputStream, daemon: Rc<Daemon>) {
    // read_line_utf8_async, not read_line_async: the byte-slice
    // variant's gio-rs trampoline reads an *uninitialized* length when
    // the stream ends with no line (client connected and closed), and
    // its slice assertion then aborts the daemon from a C callback
    // that cannot unwind. The utf8 variant passes a null length
    // pointer and models EOF as Ok(None). Requests are JSON, so utf8
    // is not a restriction.
    // Automation shares GTK's main context with the visible browser. Keep
    // socket readiness below input, frame, and WebKit callbacks so a burst of
    // agent commands cannot make keyboard or pointer events wait behind IPC.
    input.clone().read_line_utf8_async(
        glib::Priority::DEFAULT_IDLE,
        gio::Cancellable::NONE,
        move |res| {
            let line = match res {
                Ok(Some(l)) => l.to_string(),
                // EOF before the next line (port scan, one-shot client
                // disconnect, dead persistent client) or read error:
                // nothing to answer.
                Ok(None) | Err(_) => return,
            };
            let request = serde_json::from_str::<Request>(line.trim());
            // Subscriptions keep the connection as an event stream: hand it
            // to the broker instead of the request/response loop. Everything
            // else (including parse errors) gets one response line, then the
            // loop waits for the next request or EOF.
            let request = match request {
                Ok(Request::Subscribe { kinds, window }) => {
                    return crate::events::subscribe(&daemon, conn, input, kinds, window);
                }
                other => other,
            };
            let conn_for_reply = conn.clone();
            let input_for_reply = input.clone();
            let daemon_for_reply = daemon.clone();
            let reply: automation::Reply = Box::new(move |response: Response| {
                let mut out = serde_json::to_vec(&response).unwrap_or_default();
                out.push(b'\n');
                let stream = conn_for_reply.output_stream();
                let conn_for_next = conn_for_reply.clone();
                let input_for_next = input_for_reply.clone();
                let daemon_for_next = daemon_for_reply.clone();
                stream.write_all_async(
                    out,
                    glib::Priority::DEFAULT,
                    gio::Cancellable::NONE,
                    move |res| {
                        // Keep the connection strictly sequential: the next request
                        // is not read until this response has finished writing. That
                        // preserves correlation for clients that pipeline by order,
                        // including deferred automation replies.
                        if res.is_ok() {
                            read_next_request(conn_for_next, input_for_next, daemon_for_next);
                        } else {
                            let _ = conn_for_next.close(gio::Cancellable::NONE);
                        }
                    },
                );
            });
            match request {
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
    let req = match req {
        Request::Batch { actions } => return dispatch_batch(daemon.clone(), actions, reply),
        other => other,
    };

    // Async paths hand the reply off and return.
    match req {
        Request::Eval { id, js, timeout_ms } => {
            return automation::eval(daemon, id, js, timeout_ms, reply);
        }
        Request::Navigate {
            id,
            url,
            wait,
            until,
            timeout_ms,
        } => {
            return automation::navigate(daemon, id, url, wait, until, timeout_ms, reply);
        }
        Request::Screenshot { id, path, full } => {
            return automation::screenshot(daemon, id, path, full, reply);
        }
        Request::WaitLoad {
            id,
            until,
            timeout_ms,
        } => {
            return automation::wait_load(daemon, id, until, timeout_ms, reply);
        }
        Request::Check {
            url,
            render,
            base,
            eval,
            shot,
            shot_path,
            full,
            baseline,
            tolerance,
            heatmap,
            until,
            keep,
            timeout_ms,
            viewports,
            baseline_dir,
        } => {
            return automation::check(
                daemon,
                url,
                render,
                base,
                eval,
                shot,
                shot_path,
                full,
                baseline,
                tolerance,
                heatmap,
                until,
                keep,
                timeout_ms,
                viewports,
                baseline_dir,
                reply,
            );
        }
        Request::Prefetch { url } => {
            return automation::prefetch(daemon, url, reply);
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
        Request::Snapshot {
            id,
            diff,
            rect,
            timeout_ms,
        } => {
            return automation::snapshot(daemon, id, diff, rect, timeout_ms, reply);
        }
        Request::Expect {
            id,
            selector,
            nth,
            contains,
            text,
            absent,
            visible,
            timeout_ms,
            watch,
        } => {
            return if watch {
                automation::expect_watch(
                    daemon, id, selector, nth, contains, text, absent, visible, reply,
                )
            } else {
                automation::expect(
                    daemon, id, selector, nth, contains, text, absent, visible, timeout_ms, reply,
                )
            };
        }
        Request::Click {
            id,
            selector,
            nth,
            contains,
            r#ref,
            trusted,
            timeout_ms,
        } => {
            return automation::click(
                daemon, id, selector, nth, contains, r#ref, trusted, timeout_ms, reply,
            );
        }
        Request::Type {
            id,
            selector,
            nth,
            contains,
            r#ref,
            text,
            trusted,
            clear,
            enter,
            timeout_ms,
        } => {
            return automation::type_text(
                daemon, id, selector, nth, contains, r#ref, text, trusted, clear, enter,
                timeout_ms, reply,
            );
        }
        Request::Motion {
            id,
            observe,
            observe_ms,
            timeout_ms,
        } => {
            if observe {
                return crate::observe::motion_observe(daemon, id, observe_ms, timeout_ms, reply);
            }
            return crate::verify::motion(daemon, id, timeout_ms, reply);
        }
        Request::Resize { id, width, height } => {
            return crate::verify::resize(daemon, id, width, height, reply);
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
        Request::Clock {
            id,
            action,
            ms,
            seed,
            timeout_ms,
        } => {
            return crate::clock::clock(daemon, id, action, ms, seed, timeout_ms, reply);
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
        Request::Net { id, clear, limit } => automation::net(daemon, id, clear, limit),
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
            // Display-free mode has no session display to show a
            // window on: the managed headless compositor renders to
            // nothing a human can see. A structured error beats
            // silently "focusing" into the void.
            if crate::compositor::display_free() {
                reply(Response::err(format!(
                    "no display: hwatud is running display-free (headless child \
                     compositor); window {id} cannot be shown. Start hwatud in a \
                     graphical session to focus windows."
                )));
                return;
            }
            let win = daemon.windows.borrow().get(&id).cloned();
            match win {
                Some(w) => {
                    // Focus promotes any window to normal: an agent (or
                    // user) explicitly asking for a background/headless
                    // window means they want to see it now.
                    w.present();
                    daemon.last_target.replace(Some(id));
                    daemon.events.emit(
                        "window",
                        Some(id),
                        serde_json::json!({ "state": "focused" }),
                    );
                    Response::ok()
                }
                None => Response::err(format!("no window {id}")),
            }
        }
        Request::Unfocus { id } => {
            let win = daemon.windows.borrow().get(&id).cloned();
            match win {
                Some(w) => {
                    w.unfocus();
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
        | Request::Check { .. }
        | Request::Prefetch { .. }
        | Request::Challenge { .. }
        | Request::Upload { .. }
        | Request::Scroll { .. }
        | Request::Snapshot { .. }
        | Request::Click { .. }
        | Request::Type { .. }
        | Request::Motion { .. }
        | Request::Seek { .. }
        | Request::Clock { .. }
        | Request::Diff { .. }
        | Request::Resize { .. }
        | Request::Expect { .. } => Response::err("internal: async request in sync path"),
        // Handled above; reaching here means an internal misroute.
        Request::Batch { .. } => Response::err("internal: batch in sync path"),
        // Subscribe is intercepted in handle_conn (it keeps the
        // connection); reaching dispatch means an internal misroute.
        Request::Subscribe { .. } => Response::err("internal: subscribe in one-shot path"),
    };
    reply(response);
}

fn dispatch_batch(daemon: Rc<Daemon>, actions: Vec<Request>, reply: automation::Reply) {
    if let Err(e) = Request::validate_batch(&actions) {
        reply(Response::err(format!("bad batch: {e}")));
        return;
    }
    let actions = Rc::new(actions);
    let steps = Rc::new(RefCell::new(Vec::with_capacity(actions.len())));
    let final_reply = Rc::new(RefCell::new(Some(reply)));
    dispatch_batch_step(daemon, actions, steps, final_reply, 0);
}

fn dispatch_batch_step(
    daemon: Rc<Daemon>,
    actions: Rc<Vec<Request>>,
    steps: Rc<RefCell<Vec<BatchStepResult>>>,
    final_reply: Rc<RefCell<Option<automation::Reply>>>,
    index: usize,
) {
    if index >= actions.len() {
        finish_batch(actions, steps, final_reply, None);
        return;
    }

    let action = actions[index].clone();
    let action_name = action.kind().to_string();
    let daemon_next = daemon.clone();
    let actions_next = actions.clone();
    let steps_next = steps.clone();
    let final_reply_next = final_reply.clone();
    dispatch(
        &daemon,
        action,
        Box::new(move |response| {
            let error = match &response {
                Response::Err { message } => Some(message.clone()),
                Response::Ok { .. } => None,
            };
            let failed = error.is_some();
            steps_next.borrow_mut().push(BatchStepResult {
                index,
                action: action_name,
                status: if failed {
                    BatchStepStatus::Error
                } else {
                    BatchStepStatus::Ok
                },
                response: Some(response),
                error,
                skipped_reason: None,
            });
            if failed {
                finish_batch(actions_next, steps_next, final_reply_next, Some(index));
            } else {
                dispatch_batch_step(
                    daemon_next,
                    actions_next,
                    steps_next,
                    final_reply_next,
                    index + 1,
                );
            }
        }),
    );
}

fn finish_batch(
    actions: Rc<Vec<Request>>,
    steps: Rc<RefCell<Vec<BatchStepResult>>>,
    final_reply: Rc<RefCell<Option<automation::Reply>>>,
    failed_at: Option<usize>,
) {
    let mut steps = steps.borrow_mut();
    if let Some(failed_at) = failed_at {
        for index in failed_at + 1..actions.len() {
            steps.push(BatchStepResult {
                index,
                action: actions[index].kind().to_string(),
                status: BatchStepStatus::Skipped,
                response: None,
                error: None,
                skipped_reason: Some(format!("not run after step {failed_at} failed")),
            });
        }
    }
    let result = BatchResult {
        complete: failed_at.is_none(),
        executed: failed_at.map_or(actions.len(), |i| i + 1),
        failed_at,
        steps: std::mem::take(&mut *steps),
    };
    drop(steps);
    let Some(reply) = final_reply.borrow_mut().take() else {
        return;
    };
    reply(Response::value(serde_json::json!({ "batch": result })));
}

/// Turn bar/CLI input into a loadable URL: explicit schemes and
/// `about:` pass through, existing local paths become `file://` URLs,
/// bare hosts get `https://` (`http://` for
/// loopback: `localhost`, `*.localhost`, `127.*`, `[::1]`, since local
/// dev servers rarely speak TLS), and anything that doesn't look like
/// a URL becomes a web search with the configured engine (see
/// [`crate::search`]). Shared with the in-window URL bar so both
/// entry points resolve input identically.
pub fn normalize_url(input: String) -> String {
    let input = input.trim().to_string();
    if input.contains("://") || input.starts_with("about:") {
        input
    } else if let Ok(path) = std::fs::canonicalize(&input) {
        glib::filename_to_uri(path, None)
            .map(String::from)
            .unwrap_or_else(|_| crate::search::url_for(&input))
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
    use super::{finish_batch, normalize_url};
    use hwatu_ipc::{BatchStepResult, BatchStepStatus, Request, Response};
    use std::cell::RefCell;
    use std::rc::Rc;

    fn snapshot_request() -> Request {
        Request::Snapshot {
            id: None,
            diff: false,
            rect: false,
            timeout_ms: None,
        }
    }

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

    #[test]
    fn existing_local_paths_become_file_urls() {
        let path =
            std::env::temp_dir().join(format!("hwatu local path {}.html", std::process::id()));
        std::fs::write(&path, "<title>local</title>").unwrap();

        let normalized = normalize_url(path.to_string_lossy().into_owned());
        let expected = glib::filename_to_uri(std::fs::canonicalize(&path).unwrap(), None)
            .unwrap()
            .to_string();
        assert_eq!(normalized, expected);
        assert!(normalized.starts_with("file://"));
        assert!(normalized.contains("%20"));

        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn daemon_batch_validation_rejects_before_execution() {
        let actions = vec![Request::Close { id: 1 }];
        assert!(Request::validate_batch(&actions)
            .unwrap_err()
            .contains("unsupported"));

        let actions = vec![Request::Batch {
            actions: vec![snapshot_request()],
        }];
        assert!(Request::validate_batch(&actions)
            .unwrap_err()
            .contains("nested"));
    }

    #[test]
    fn finish_batch_records_explicit_partial_execution() {
        let actions = Rc::new(vec![
            snapshot_request(),
            Request::Click {
                id: None,
                selector: Some("button".into()),
                nth: None,
                contains: None,
                r#ref: None,
                trusted: false,
                timeout_ms: None,
            },
            Request::Type {
                id: None,
                selector: Some("input".into()),
                nth: None,
                contains: None,
                r#ref: None,
                text: "x".into(),
                trusted: false,
                clear: true,
                enter: false,
                timeout_ms: None,
            },
        ]);
        let steps = Rc::new(RefCell::new(vec![
            BatchStepResult {
                index: 0,
                action: "snapshot".into(),
                status: BatchStepStatus::Ok,
                response: Some(Response::ok()),
                error: None,
                skipped_reason: None,
            },
            BatchStepResult {
                index: 1,
                action: "click".into(),
                status: BatchStepStatus::Error,
                response: Some(Response::err("button not found")),
                error: Some("button not found".into()),
                skipped_reason: None,
            },
        ]));
        let captured = Rc::new(RefCell::new(None));
        let captured_reply = captured.clone();
        let reply = Box::new(move |response| {
            *captured_reply.borrow_mut() = Some(response);
        });
        finish_batch(actions, steps, Rc::new(RefCell::new(Some(reply))), Some(1));

        let Response::Ok { value: Some(v), .. } = captured.borrow_mut().take().unwrap() else {
            panic!("expected ok batch response");
        };
        assert_eq!(v["batch"]["complete"], false);
        assert_eq!(v["batch"]["executed"], 2);
        assert_eq!(v["batch"]["failed_at"], 1);
        assert_eq!(v["batch"]["steps"][2]["status"], "skipped");
        assert_eq!(v["batch"]["steps"][2]["action"], "type");
    }
}
