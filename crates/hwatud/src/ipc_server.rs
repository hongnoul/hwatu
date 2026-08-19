// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! Bounded Unix/TCP IPC server integrated with the GLib main loop so all
//! window work happens on the GTK main thread.

use crate::{adblock::Adblock, automation, window::BrowserWindow, Daemon};
use gtk::gio;
use gtk::gio::prelude::*;
use gtk::glib;
use hwatu_ipc::{
    AdblockCmd, AuthReply, AuthRequest, BatchResult, BatchStepResult, BatchStepStatus, Request,
    Response,
};
use std::cell::RefCell;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use subtle::ConstantTimeEq;

const MAX_TCP_CONNECTIONS: u32 = 64;
const READ_CHUNK_BYTES: usize = 8 * 1024;
const AUTH_TIMEOUT: Duration = Duration::from_secs(5);

static TCP_CONNECTIONS: AtomicU32 = AtomicU32::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransportKind {
    Unix,
    Tcp,
}

struct TcpPermit;

impl TcpPermit {
    fn acquire() -> Option<Self> {
        TCP_CONNECTIONS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                (current < MAX_TCP_CONNECTIONS).then_some(current + 1)
            })
            .ok()
            .map(|_| Self)
    }
}

impl Drop for TcpPermit {
    fn drop(&mut self) {
        TCP_CONNECTIONS.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(crate) struct ServerConnection {
    pub(crate) socket: gio::SocketConnection,
    pub(crate) transport: TransportKind,
    _permit: Option<TcpPermit>,
}

struct FrameBuffer(Vec<u8>);

enum BufferedFrame {
    Ready(Vec<u8>),
    NeedMore,
    TooLarge,
}

impl FrameBuffer {
    fn take(&mut self, max_bytes: usize) -> BufferedFrame {
        if let Some(newline) = self.0.iter().position(|byte| *byte == b'\n') {
            if newline + 1 > max_bytes {
                return BufferedFrame::TooLarge;
            }
            let remainder = self.0.split_off(newline + 1);
            let mut frame = std::mem::replace(&mut self.0, remainder);
            frame.pop();
            return BufferedFrame::Ready(frame);
        }
        if self.0.len() >= max_bytes {
            BufferedFrame::TooLarge
        } else {
            BufferedFrame::NeedMore
        }
    }
}

pub(crate) struct FrameReader {
    connection: Rc<ServerConnection>,
    buffer: RefCell<FrameBuffer>,
}

impl FrameReader {
    fn new(connection: Rc<ServerConnection>) -> Rc<Self> {
        Rc::new(Self {
            connection,
            buffer: RefCell::new(FrameBuffer(Vec::new())),
        })
    }
}

#[derive(Debug)]
enum FrameReadError {
    TooLarge,
    Truncated,
    Io,
}

type FrameCallback = Box<dyn FnOnce(Result<Option<Vec<u8>>, FrameReadError>)>;

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

    if let Some(configured) = daemon.security.tcp_listen.as_deref() {
        let socket_address = parse_tcp_listen(configured)?;
        let ip = gio::InetAddress::from_string(&socket_address.ip().to_string())
            .ok_or_else(|| std::io::Error::other("could not construct TCP listen address"))?;
        let address = gio::InetSocketAddress::new(&ip, socket_address.port());
        listener
            .add_address(
                &address,
                gio::SocketType::Stream,
                gio::SocketProtocol::Tcp,
                glib::Object::NONE,
            )
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        println!("hwatud: authenticated TCP listening on {socket_address}");
    }

    accept_next(listener, daemon);
    Ok(())
}

fn parse_tcp_listen(configured: &str) -> std::io::Result<SocketAddr> {
    let address = if let Ok(port) = configured.parse::<u16>() {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
    } else {
        configured.parse::<SocketAddr>().map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("invalid TCP listener {configured:?}; use port or numeric loopback:port"),
            )
        })?
    };
    if address.port() == 0 || !address.ip().is_loopback() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "TCP listener must use a nonzero port on a loopback address",
        ));
    }
    Ok(address)
}

fn accept_next(listener: gio::SocketListener, daemon: Rc<Daemon>) {
    listener
        .clone()
        .accept_async(gio::Cancellable::NONE, move |res| {
            if let Ok((socket, _)) = res {
                let transport = if matches!(
                    socket.socket().family(),
                    gio::SocketFamily::Ipv4 | gio::SocketFamily::Ipv6
                ) {
                    TransportKind::Tcp
                } else {
                    TransportKind::Unix
                };
                let permit = if transport == TransportKind::Tcp {
                    let Some(permit) = TcpPermit::acquire() else {
                        let _ = socket.close(gio::Cancellable::NONE);
                        accept_next(listener, daemon);
                        return;
                    };
                    let _ = socket.socket().set_option(6, 1, 1);
                    Some(permit)
                } else {
                    None
                };
                let connection = Rc::new(ServerConnection {
                    socket,
                    transport,
                    _permit: permit,
                });
                handle_conn(connection, daemon.clone());
            }
            accept_next(listener, daemon);
        });
}

fn handle_conn(connection: Rc<ServerConnection>, daemon: Rc<Daemon>) {
    let reader = FrameReader::new(connection);
    if reader.connection.transport == TransportKind::Tcp {
        authenticate(reader, daemon);
    } else {
        read_next_request(reader, daemon);
    }
}

fn read_bounded_frame(
    reader: Rc<FrameReader>,
    max_bytes: usize,
    cancellable: Option<gio::Cancellable>,
    callback: FrameCallback,
) {
    match reader.buffer.borrow_mut().take(max_bytes) {
        BufferedFrame::Ready(frame) => return callback(Ok(Some(frame))),
        BufferedFrame::TooLarge => return callback(Err(FrameReadError::TooLarge)),
        BufferedFrame::NeedMore => {}
    }

    let remaining = max_bytes - reader.buffer.borrow().0.len();
    let input = reader.connection.socket.input_stream();
    let cancellable_for_read = cancellable.clone();
    input.read_bytes_async(
        remaining.min(READ_CHUNK_BYTES),
        glib::Priority::DEFAULT_IDLE,
        cancellable_for_read.as_ref(),
        move |res| match res {
            Ok(bytes) if bytes.is_empty() => {
                let result = if reader.buffer.borrow().0.is_empty() {
                    Ok(None)
                } else {
                    Err(FrameReadError::Truncated)
                };
                callback(result);
            }
            Ok(bytes) => {
                reader.buffer.borrow_mut().0.extend_from_slice(&bytes);
                read_bounded_frame(reader, max_bytes, cancellable, callback);
            }
            Err(_) => callback(Err(FrameReadError::Io)),
        },
    );
}

fn authenticate(reader: Rc<FrameReader>, daemon: Rc<Daemon>) {
    let cancellable = gio::Cancellable::new();
    let timer = Rc::new(RefCell::new(None));
    let timer_for_timeout = timer.clone();
    let cancellable_for_timeout = cancellable.clone();
    let source = glib::timeout_add_local_once(AUTH_TIMEOUT, move || {
        timer_for_timeout.borrow_mut().take();
        cancellable_for_timeout.cancel();
    });
    *timer.borrow_mut() = Some(source);

    read_bounded_frame(
        reader.clone(),
        hwatu_ipc::MAX_AUTH_FRAME_BYTES,
        Some(cancellable),
        Box::new(move |result| {
            if let Some(source) = timer.borrow_mut().take() {
                source.remove();
            }
            let authenticated = result
                .ok()
                .flatten()
                .and_then(|frame| serde_json::from_slice::<AuthRequest>(&frame).ok())
                .is_some_and(|request| {
                    daemon.security.tcp_token.as_ref().is_some_and(|secret| {
                        tokens_equal(request.token.as_bytes(), secret.as_bytes())
                    })
                });
            if authenticated {
                write_auth_reply(reader, daemon, AuthReply::Ok, true);
            } else {
                write_auth_reply(
                    reader,
                    daemon,
                    AuthReply::Err {
                        message: "authentication failed".to_string(),
                    },
                    false,
                );
            }
        }),
    );
}

fn tokens_equal(candidate: &[u8], expected: &[u8]) -> bool {
    bool::from(candidate.ct_eq(expected))
}

fn write_auth_reply(reader: Rc<FrameReader>, daemon: Rc<Daemon>, reply: AuthReply, proceed: bool) {
    let Ok(frame) = hwatu_ipc::encode_frame(&reply, hwatu_ipc::MAX_AUTH_FRAME_BYTES) else {
        let _ = reader.connection.socket.close(gio::Cancellable::NONE);
        return;
    };
    let output = reader.connection.socket.output_stream();
    output.write_all_async(
        frame,
        glib::Priority::DEFAULT,
        gio::Cancellable::NONE,
        move |result| {
            if proceed && result.is_ok() {
                read_next_request(reader, daemon);
            } else {
                let _ = reader.connection.socket.close(gio::Cancellable::NONE);
            }
        },
    );
}

fn read_next_request(reader: Rc<FrameReader>, daemon: Rc<Daemon>) {
    read_bounded_frame(
        reader.clone(),
        hwatu_ipc::MAX_FRAME_BYTES,
        None,
        Box::new(move |result| {
            let frame = match result {
                Ok(Some(frame)) => frame,
                Ok(None) | Err(FrameReadError::Truncated | FrameReadError::Io) => return,
                Err(FrameReadError::TooLarge) => {
                    write_response(
                        reader,
                        daemon,
                        Response::err("request frame exceeds protocol limit"),
                        false,
                    );
                    return;
                }
            };
            let request = serde_json::from_slice::<Request>(&frame);
            // Subscriptions keep the connection as an event stream: hand it
            // to the broker instead of the request/response loop. Everything
            // else (including parse errors) gets one response line, then the
            // loop waits for the next request or EOF.
            let request = match request {
                Ok(Request::Subscribe { kinds, window }) => {
                    return crate::events::subscribe(
                        &daemon,
                        reader.connection.clone(),
                        kinds,
                        window,
                    );
                }
                other => other,
            };
            let transport = reader.connection.transport;
            let daemon_for_reply = daemon.clone();
            let reply: automation::Reply = Box::new(move |response: Response| {
                write_response(reader, daemon_for_reply, response, true);
            });
            match request {
                Ok(req) => dispatch(&daemon, req, transport, reply),
                Err(e) => reply(Response::err(format!("bad request: {e}"))),
            }
        }),
    );
}

fn write_response(reader: Rc<FrameReader>, daemon: Rc<Daemon>, response: Response, proceed: bool) {
    let frame = hwatu_ipc::encode_frame(&response, hwatu_ipc::MAX_FRAME_BYTES).or_else(|_| {
        hwatu_ipc::encode_frame(
            &Response::err("response frame exceeds protocol limit"),
            hwatu_ipc::MAX_FRAME_BYTES,
        )
    });
    let Ok(frame) = frame else {
        let _ = reader.connection.socket.close(gio::Cancellable::NONE);
        return;
    };
    let output = reader.connection.socket.output_stream();
    output.write_all_async(
        frame,
        glib::Priority::DEFAULT,
        gio::Cancellable::NONE,
        move |result| {
            if proceed && result.is_ok() {
                read_next_request(reader, daemon);
            } else {
                let _ = reader.connection.socket.close(gio::Cancellable::NONE);
            }
        },
    );
}

/// Unix seconds now (hand-off queue timestamps).
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---- client fairness (platform item 6b) ------------------------------
//
// Cheap bulkheads before parallel-agent use gets heavy: one runaway
// agent must not starve the daemon. Two bounds, both answering
// structured errors instead of silently queueing:
//
// - window quota: `Open` requests beyond HWATU_MAX_WINDOWS (default
//   64) are refused. Windows are the expensive resource (a web
//   process each); checks/prefetches are already pool-capped.
// - request rate: a global token bucket (HWATU_MAX_RPS sustained,
//   default 200/s, burst 2x) across all connections. Generous enough
//   that no legitimate workload notices; a tight fire loop hits it.

thread_local! {
    static RATE: std::cell::RefCell<(f64, std::time::Instant)> =
        std::cell::RefCell::new((max_rps() * 2.0, std::time::Instant::now()));
}

fn max_windows() -> usize {
    std::env::var("HWATU_MAX_WINDOWS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|n: &usize| *n > 0)
        .unwrap_or(64)
}

fn max_rps() -> f64 {
    std::env::var("HWATU_MAX_RPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|n: &f64| *n > 0.0)
        .unwrap_or(200.0)
}

/// Take one token; false = over the rate bound.
fn rate_admit() -> bool {
    RATE.with(|cell| {
        let (ref mut tokens, ref mut last) = *cell.borrow_mut();
        let rps = max_rps();
        let cap = rps * 2.0;
        let now = std::time::Instant::now();
        *tokens = (*tokens + now.duration_since(*last).as_secs_f64() * rps).min(cap);
        *last = now;
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            true
        } else {
            false
        }
    })
}

/// Clear stored site data (roadmap H16). WebKit's clear() is
/// type-based and whole-store; per-host removal fetches matching
/// WebsiteData records first. Site-store decisions and (on full
/// clears) history go with it: "clear everything about this site"
/// must mean everything hwatu remembers, not just cookies.
fn clear_site_data(daemon: &Rc<Daemon>, host: Option<String>, reply: automation::Reply) {
    let Some(session) =
        webkit6::NetworkSession::default().or_else(|| daemon.network_session.clone())
    else {
        reply(Response::err("no network session".to_string()));
        return;
    };
    let Some(manager) = session.website_data_manager() else {
        reply(Response::err("no website data manager".to_string()));
        return;
    };
    let types = webkit6::WebsiteDataTypes::all();
    // hwatu-side memory first (synchronous, always succeeds).
    let decisions = daemon.site_store.clear_permissions(host.as_deref());
    let history = if host.is_none() {
        daemon.history.clear()
    } else {
        0
    };

    // WebKit's clear/remove callbacks demand Send, but Reply is a
    // main-thread closure. The callbacks do run on this same GLib
    // main context; bridge the result through a channel and poll it
    // locally so the non-Send reply never crosses the bound.
    let (tx, rx) = std::sync::mpsc::channel::<Result<serde_json::Value, String>>();
    glib::timeout_add_local(std::time::Duration::from_millis(25), {
        let mut reply = Some(reply);
        move || match rx.try_recv() {
            Ok(result) => {
                if let Some(reply) = reply.take() {
                    match result {
                        Ok(value) => reply(Response::value(value)),
                        Err(message) => reply(Response::err(message)),
                    }
                }
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                if let Some(reply) = reply.take() {
                    reply(Response::err("clear-site-data lost its result".to_string()));
                }
                glib::ControlFlow::Break
            }
        }
    });

    match host {
        None => {
            manager.clear(
                types,
                glib::TimeSpan::from_seconds(0),
                gtk::gio::Cancellable::NONE,
                move |result| {
                    let _ = tx.send(match result {
                        Ok(()) => Ok(serde_json::json!({
                            "cleared": "all",
                            "decisions_dropped": decisions,
                            "history_dropped": history,
                        })),
                        Err(e) => Err(format!("clear failed: {e}")),
                    });
                },
            );
        }
        Some(host) => {
            // Fetch, filter by registrable-domain match, remove.
            let manager2 = manager.clone();
            manager.fetch(types, gtk::gio::Cancellable::NONE, move |result| {
                let records = match result {
                    Ok(records) => records,
                    Err(e) => {
                        let _ = tx.send(Err(format!("fetch failed: {e}")));
                        return;
                    }
                };
                let bare = host.strip_prefix("www.").unwrap_or(&host).to_string();
                let matching: Vec<webkit6::WebsiteData> = records
                    .into_iter()
                    .filter(|record| {
                        record.name().is_some_and(|name| {
                            let name = name.to_lowercase();
                            name == bare || name == host || name.ends_with(&format!(".{bare}"))
                        })
                    })
                    .collect();
                if matching.is_empty() {
                    let _ = tx.send(Ok(serde_json::json!({
                        "cleared": 0,
                        "host": host,
                        "decisions_dropped": decisions,
                    })));
                    return;
                }
                let count = matching.len();
                let refs: Vec<&webkit6::WebsiteData> = matching.iter().collect();
                manager2.remove(types, &refs, gtk::gio::Cancellable::NONE, move |result| {
                    let _ = tx.send(match result {
                        Ok(()) => Ok(serde_json::json!({
                            "cleared": count,
                            "host": host,
                            "decisions_dropped": decisions,
                        })),
                        Err(e) => Err(format!("remove failed: {e}")),
                    });
                });
            });
        }
    }
}

/// Route one request. Most commands answer synchronously; the
/// automation commands (eval/navigate/screenshot/wait_load) complete
/// later on the main loop and consume `reply` when they finish.
fn dispatch(daemon: &Rc<Daemon>, req: Request, transport: TransportKind, reply: automation::Reply) {
    if !daemon.security.eval_enabled && req.uses_eval() {
        reply(Response::err(
            "eval disabled by daemon policy (--no-eval)".to_string(),
        ));
        return;
    }

    // Client fairness (platform item 6b): rate bulkhead first — a
    // structured "over rate" error, never silent queueing. Ping is
    // exempt so health checks and stale-daemon detection always work.
    if !matches!(req, Request::Ping) && !rate_admit() {
        reply(Response::err(format!(
            "over rate: daemon-wide request budget exceeded ({}/s sustained); \
             back off and retry (HWATU_MAX_RPS overrides)",
            max_rps()
        )));
        return;
    }
    // Window quota: Open is the request that allocates the expensive
    // resource. Check/render use the pooled headless windows and stay
    // bounded by their own caps.
    if matches!(req, Request::Open { .. }) {
        let open = daemon.windows.borrow().len();
        let cap = max_windows();
        if open >= cap {
            reply(Response::err(format!(
                "over quota: {open} windows open (cap {cap}); close some or \
                 raise HWATU_MAX_WINDOWS"
            )));
            return;
        }
    }

    let req = match req {
        Request::Batch { actions } => {
            return dispatch_batch(daemon.clone(), actions, transport, reply);
        }
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
        Request::Screenshot {
            id,
            path,
            full,
            data,
        } => {
            if transport == TransportKind::Tcp && (!data || path.is_some()) {
                return reply(Response::err(
                    "TCP screenshots must request inline data and cannot name a daemon path",
                ));
            }
            return automation::screenshot(daemon, id, path, full, data, reply);
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
            shot_data,
            full,
            baseline,
            baseline_data,
            tolerance,
            heatmap,
            heatmap_data,
            until,
            keep,
            timeout_ms,
            viewports,
            baseline_dir,
        } => {
            if transport == TransportKind::Tcp
                && (shot_path.is_some()
                    || baseline.is_some()
                    || heatmap.is_some()
                    || baseline_dir.is_some())
            {
                return reply(Response::err(
                    "TCP checks cannot read or write daemon-host artifact paths",
                ));
            }
            if transport == TransportKind::Tcp && shot && !shot_data {
                return reply(Response::err(
                    "TCP checks that capture a screenshot must request inline shot data",
                ));
            }
            return automation::check(
                daemon,
                url,
                render,
                base,
                eval,
                shot,
                shot_path,
                shot_data,
                full,
                baseline,
                baseline_data,
                tolerance,
                heatmap,
                heatmap_data,
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
        Request::ClearSiteData { host } => {
            return clear_site_data(daemon, host, reply);
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
            data,
            timeout_ms,
        } => {
            if transport == TransportKind::Tcp && data.is_none() {
                return reply(Response::err("TCP uploads require inline file data"));
            }
            return automation::upload(daemon, id, selector, path, data, timeout_ms, reply);
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
            budget,
            timeout_ms,
        } => {
            return automation::snapshot(daemon, id, diff, rect, budget, timeout_ms, reply);
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
        Request::Press {
            id,
            key,
            timeout_ms,
        } => {
            return automation::press(daemon, id, key, timeout_ms, reply);
        }
        Request::Paste {
            id,
            selector,
            nth,
            contains,
            r#ref,
            timeout_ms,
        } => {
            return automation::paste(
                daemon, id, selector, nth, contains, r#ref, timeout_ms, reply,
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
            baseline_data,
            tolerance,
            heatmap,
            heatmap_data,
            full,
            timeout_ms: _,
        } => {
            if transport == TransportKind::Tcp && (baseline.is_some() || heatmap.is_some()) {
                return reply(Response::err(
                    "TCP diffs cannot read or write daemon-host artifact paths",
                ));
            }
            return crate::verify::diff(
                daemon,
                id,
                other,
                baseline,
                baseline_data,
                tolerance,
                heatmap,
                heatmap_data,
                full,
                reply,
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
        Request::Jump { query, open } => {
            // Fuzzy jump (roadmap H29): open windows first, then
            // history. Window scoring reuses the history matcher's
            // spirit: substring on url/title, host-prefix boost.
            let q = query.to_lowercase();
            let best_window = {
                let windows = daemon.windows.borrow();
                let mut best: Option<(u64, i32)> = None;
                for w in windows.values() {
                    let info = w.info();
                    // Headless windows belong to agents; jumping into
                    // one would materialize an agent's workspace.
                    if info.mode == hwatu_ipc::OpenMode::Headless {
                        continue;
                    }
                    let url = info.url.to_lowercase();
                    let title = info.title.to_lowercase();
                    let host = url
                        .strip_prefix("https://")
                        .or_else(|| url.strip_prefix("http://"))
                        .unwrap_or(&url)
                        .trim_start_matches("www.");
                    let score = if host.starts_with(&q) {
                        3
                    } else if url.contains(&q) || title.contains(&q) {
                        1
                    } else {
                        continue;
                    };
                    if best.is_none_or(|(_, s)| score > s) {
                        best = Some((info.id, score));
                    }
                }
                best
            };
            if let Some((id, _)) = best_window {
                if crate::compositor::display_free() {
                    reply(Response::err(
                        "no display: hwatud is running display-free; cannot focus".to_string(),
                    ));
                    return;
                }
                let win = daemon.windows.borrow().get(&id).cloned();
                if let Some(win) = win {
                    win.present();
                    daemon.last_target.replace(Some(id));
                    reply(Response::value(serde_json::json!({
                        "jump": "focused",
                        "id": id,
                    })));
                    return;
                }
            }
            // No live window: fall back to history.
            let hits = daemon.history.complete(&query, 1);
            match hits.into_iter().next() {
                Some(hit) if open => {
                    let info = BrowserWindow::open(
                        daemon,
                        Some(hit.url.clone()),
                        None,
                        hwatu_ipc::OpenMode::Normal,
                    );
                    daemon.last_target.replace(Some(info.id));
                    reply(Response::value(serde_json::json!({
                        "jump": "opened",
                        "id": info.id,
                        "url": hit.url,
                    })));
                }
                Some(hit) => {
                    reply(Response::value(serde_json::json!({
                        "jump": "match",
                        "url": hit.url,
                        "title": hit.title,
                    })));
                }
                None => {
                    reply(Response::err(format!(
                        "no window or history match for {query:?}"
                    )));
                }
            }
            return;
        }
        Request::Handoff { id, reason, now } => {
            let Some(win) = daemon.windows.borrow().get(&id).cloned() else {
                reply(Response::err(format!("no window {id}")));
                return;
            };
            if now {
                if crate::compositor::display_free() {
                    reply(Response::err(
                        "no display: hwatud is running display-free; queue with \
                         `hwatu handoff <id> --reason ...` (without --now) instead"
                            .to_string(),
                    ));
                    return;
                }
                win.present();
                win.flash_bar(&format!("agent needs you: {reason}"), 30);
                daemon.events.emit(
                    "handoff",
                    Some(id),
                    serde_json::json!({ "state": "presented", "reason": reason }),
                );
                reply(Response::value(
                    serde_json::json!({ "handoff": "presented" }),
                ));
                return;
            }
            let mut queue = daemon.handoffs.borrow_mut();
            // Re-queueing the same window updates the reason instead
            // of duplicating the entry.
            queue.retain(|e| e.window_id != id);
            queue.push(crate::HandoffEntry {
                window_id: id,
                reason: reason.clone(),
                queued_at: unix_now(),
            });
            let position = queue.len();
            drop(queue);
            daemon.events.emit(
                "handoff",
                Some(id),
                serde_json::json!({ "state": "queued", "reason": reason }),
            );
            reply(Response::value(serde_json::json!({
                "handoff": "queued",
                "position": position,
            })));
            return;
        }
        Request::Handoffs { take } => {
            match take {
                None => {
                    let now = unix_now();
                    let queue = daemon.handoffs.borrow();
                    let entries: Vec<_> = queue
                        .iter()
                        .map(|e| {
                            serde_json::json!({
                                "id": e.window_id,
                                "reason": e.reason,
                                "queued_at": e.queued_at,
                                "waiting_secs": now.saturating_sub(e.queued_at),
                            })
                        })
                        .collect();
                    reply(Response::value(serde_json::json!({ "handoffs": entries })));
                }
                Some(id) => {
                    // Validate everything BEFORE consuming the entry: a
                    // failed take (dead window aside, e.g. display-free
                    // daemon) must leave the hand-off queued, or the
                    // human's one attempt silently discards the agent's
                    // request.
                    let exists = daemon.handoffs.borrow().iter().any(|e| e.window_id == id);
                    if !exists {
                        reply(Response::err(format!("no pending handoff for window {id}")));
                        return;
                    }
                    let win = daemon.windows.borrow().get(&id).cloned();
                    let Some(win) = win else {
                        // The window died while queued: the entry can
                        // never be taken; drop it with a clear error.
                        daemon.handoffs.borrow_mut().retain(|e| e.window_id != id);
                        reply(Response::err(format!(
                            "window {id} closed while its handoff was queued"
                        )));
                        return;
                    };
                    if crate::compositor::display_free() {
                        reply(Response::err(
                            "no display: hwatud is running display-free; cannot present"
                                .to_string(),
                        ));
                        return;
                    }
                    let entry = {
                        let mut queue = daemon.handoffs.borrow_mut();
                        let pos = queue.iter().position(|e| e.window_id == id);
                        pos.map(|p| queue.remove(p))
                    };
                    let Some(entry) = entry else {
                        reply(Response::err(format!("no pending handoff for window {id}")));
                        return;
                    };
                    let waited = unix_now().saturating_sub(entry.queued_at);
                    win.present();
                    win.flash_bar(&format!("handoff: {}", entry.reason), 30);
                    // Queued-at/answered-at logged: the cost of waiting
                    // on a human is a measured number, not a vibe.
                    println!(
                        "hwatud: handoff for window {id} answered after {waited}s ({})",
                        entry.reason
                    );
                    daemon.events.emit(
                        "handoff",
                        Some(id),
                        serde_json::json!({
                            "state": "taken",
                            "reason": entry.reason,
                            "waited_secs": waited,
                        }),
                    );
                    reply(Response::value(serde_json::json!({
                        "handoff": "taken",
                        "waited_secs": waited,
                    })));
                }
            }
            return;
        }
        Request::History {
            query,
            limit,
            clear,
        } => {
            if clear {
                let removed = daemon.history.clear();
                Response::value(serde_json::json!({ "cleared": removed }))
            } else {
                let hits = daemon
                    .history
                    .complete(&query, limit.unwrap_or(20).min(100));
                let entries: Vec<_> = hits
                    .into_iter()
                    .map(|h| {
                        serde_json::json!({
                            "url": h.url,
                            "title": h.title,
                            "score": h.score,
                        })
                    })
                    .collect();
                Response::value(serde_json::json!({ "history": entries }))
            }
        }
        Request::Open {
            url,
            app_id,
            mode,
            profile,
        } => {
            let url = url.map(normalize_url);
            let info = BrowserWindow::open_with_profile(daemon, url, app_id, mode, profile);
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
            let daemon = daemon.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(50), move || {
                // Clean quit: default is no resurrection next start.
                // `"restore_session": true` (roadmap H19) opts into
                // restoring even after intentional exits — the WM-
                // workspace crowd treats the browser session as
                // durable state, not a per-run artifact.
                let restore_on_quit = crate::window::config_value("restore_session")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if restore_on_quit {
                    daemon.save_session_now();
                } else {
                    crate::session::clear();
                }
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
        | Request::Press { .. }
        | Request::Paste { .. }
        | Request::Motion { .. }
        | Request::Seek { .. }
        | Request::Clock { .. }
        | Request::Diff { .. }
        | Request::Resize { .. }
        | Request::ClearSiteData { .. }
        | Request::Expect { .. } => Response::err("internal: async request in sync path"),
        // Handled above; reaching here means an internal misroute.
        Request::Batch { .. } => Response::err("internal: batch in sync path"),
        // Subscribe is intercepted in handle_conn (it keeps the
        // connection); reaching dispatch means an internal misroute.
        Request::Subscribe { .. } => Response::err("internal: subscribe in one-shot path"),
    };
    reply(response);
}

fn dispatch_batch(
    daemon: Rc<Daemon>,
    actions: Vec<Request>,
    transport: TransportKind,
    reply: automation::Reply,
) {
    if let Err(e) = Request::validate_batch(&actions) {
        reply(Response::err(format!("bad batch: {e}")));
        return;
    }
    let inline_outputs: usize = actions.iter().map(inline_output_count).sum();
    if inline_outputs > 1 {
        reply(Response::err(
            "bad batch: at most one inline screenshot or heatmap output is allowed per batch",
        ));
        return;
    }
    let actions = Rc::new(actions);
    let steps = Rc::new(RefCell::new(Vec::with_capacity(actions.len())));
    let final_reply = Rc::new(RefCell::new(Some(reply)));
    dispatch_batch_step(daemon, actions, steps, final_reply, transport, 0);
}

fn inline_output_count(request: &Request) -> usize {
    match request {
        Request::Screenshot { data, .. } => usize::from(*data),
        Request::Check {
            shot_data,
            heatmap_data,
            ..
        } => usize::from(*shot_data) + usize::from(*heatmap_data),
        Request::Diff { heatmap_data, .. } => usize::from(*heatmap_data),
        Request::Batch { actions } => actions.iter().map(inline_output_count).sum(),
        _ => 0,
    }
}

fn dispatch_batch_step(
    daemon: Rc<Daemon>,
    actions: Rc<Vec<Request>>,
    steps: Rc<RefCell<Vec<BatchStepResult>>>,
    final_reply: Rc<RefCell<Option<automation::Reply>>>,
    transport: TransportKind,
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
        transport,
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
                    transport,
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
    use super::{
        finish_batch, inline_output_count, normalize_url, parse_tcp_listen, tokens_equal,
        BufferedFrame, FrameBuffer,
    };
    use hwatu_ipc::{BatchStepResult, BatchStepStatus, Request, Response};
    use std::cell::RefCell;
    use std::rc::Rc;

    fn snapshot_request() -> Request {
        Request::Snapshot {
            id: None,
            diff: false,
            rect: false,
            budget: None,
            timeout_ms: None,
        }
    }

    #[test]
    fn batch_inline_output_count_prevents_aggregate_frame_overflow() {
        let screenshot = || Request::Screenshot {
            id: None,
            path: None,
            full: false,
            data: true,
        };
        let actions = [screenshot(), screenshot()];
        assert_eq!(actions.iter().map(inline_output_count).sum::<usize>(), 2);
    }

    #[test]
    fn bounded_frame_buffer_handles_fragmentation_and_pipelining() {
        let mut buffer = FrameBuffer(br#"{"cmd":"pi"#.to_vec());
        assert!(matches!(buffer.take(64), BufferedFrame::NeedMore));
        buffer.0.extend_from_slice(b"ng\"}\nnext\n");
        let BufferedFrame::Ready(first) = buffer.take(64) else {
            panic!("first frame was not ready");
        };
        assert_eq!(first, br#"{"cmd":"ping"}"#);
        let BufferedFrame::Ready(second) = buffer.take(64) else {
            panic!("pipelined frame was not retained");
        };
        assert_eq!(second, b"next");
    }

    #[test]
    fn bounded_frame_buffer_rejects_missing_or_late_delimiters() {
        let mut exact_without_newline = FrameBuffer(vec![b'x'; 8]);
        assert!(matches!(
            exact_without_newline.take(8),
            BufferedFrame::TooLarge
        ));

        let mut newline_too_late = FrameBuffer(b"12345678\n".to_vec());
        assert!(matches!(newline_too_late.take(8), BufferedFrame::TooLarge));

        let mut exact = FrameBuffer(b"1234567\n".to_vec());
        assert!(matches!(exact.take(8), BufferedFrame::Ready(_)));
    }

    #[test]
    fn tcp_listener_accepts_only_numeric_loopback() {
        assert_eq!(
            parse_tcp_listen("8741").unwrap(),
            "127.0.0.1:8741".parse().unwrap()
        );
        assert!(parse_tcp_listen("127.0.0.1:8741").is_ok());
        assert!(parse_tcp_listen("[::1]:8741").is_ok());
        assert!(parse_tcp_listen("0").is_err());
        assert!(parse_tcp_listen("0.0.0.0:8741").is_err());
        assert!(parse_tcp_listen("192.0.2.1:8741").is_err());
        assert!(parse_tcp_listen("localhost:8741").is_err());
    }

    #[test]
    fn bearer_token_comparison_checks_full_bytes() {
        let token = b"0123456789abcdef0123456789abcdef";
        assert!(tokens_equal(token, token));
        assert!(!tokens_equal(token, b"0123456789abcdef0123456789abcdeg"));
        assert!(!tokens_equal(token, b"short"));
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
