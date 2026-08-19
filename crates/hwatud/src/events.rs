// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! Push IPC: server-initiated events on persistent connections
//! (roadmap G2). A client sends `subscribe` and holds the socket
//! open; the daemon streams [`Event`]s as JSON lines until the client
//! disconnects. One-shot clients are untouched: this module only
//! handles connections that asked to subscribe.
//!
//! Invariants:
//! - Per-connection strictly monotonic `seq` (0 = the `subscribed`
//!   ack). The daemon never skips an event for a live subscriber; a
//!   subscriber it cannot keep up with is dropped whole.
//! - No daemon-side queues for dead clients: EOF/error on the
//!   connection unregisters it immediately.
//! - The GTK main loop never blocks on a stuck client: writes are
//!   async, and a write queue past [`MAX_QUEUED_BYTES`] closes the
//!   connection instead of growing.

use gtk::gio;
use gtk::gio::prelude::*;
use gtk::glib;
use hwatu_ipc::Event;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use crate::ipc_server::ServerConnection;

/// Cap on bytes queued for one subscriber beyond what the kernel
/// socket buffer absorbed. A reader this far behind is stuck, not
/// slow; drop it rather than let the queue grow.
const MAX_QUEUED_BYTES: usize = 1 << 20;

struct Subscriber {
    /// Kind filter (`None` = all kinds).
    kinds: Option<Vec<String>>,
    /// Window filter (`None` = all windows).
    window: Option<u64>,
    /// Last sequence number sent (ack = 0).
    seq: Cell<u64>,
    /// Owning this wrapper keeps the TCP connection permit until the
    /// subscriber disconnects, including after request dispatch hands it off.
    conn: Rc<ServerConnection>,
    queue: RefCell<VecDeque<Vec<u8>>>,
    queued_bytes: Cell<usize>,
    write_in_flight: Cell<bool>,
    dead: Cell<bool>,
}

impl Subscriber {
    fn wants(&self, kind: &str, window_id: Option<u64>) -> bool {
        if let Some(w) = self.window {
            if window_id != Some(w) {
                return false;
            }
        }
        if let Some(kinds) = &self.kinds {
            if !kinds.iter().any(|k| k == kind) {
                return false;
            }
        }
        true
    }
}

/// Registry of live subscribers, owned by the daemon.
#[derive(Default)]
pub struct Broker {
    subs: RefCell<Vec<Rc<Subscriber>>>,
}

impl Broker {
    /// Fan an event out to every matching subscriber. Cheap when
    /// nobody subscribed (one borrow + is_empty).
    pub fn emit(&self, kind: &str, window_id: Option<u64>, data: serde_json::Value) {
        // Clone the Rc list so subscriber callbacks can't re-enter a
        // held borrow of the registry.
        let subs: Vec<_> = {
            let subs = self.subs.borrow();
            if subs.is_empty() {
                return;
            }
            subs.clone()
        };
        let ts_ms = now_ms();
        for sub in subs {
            if sub.dead.get() || !sub.wants(kind, window_id) {
                continue;
            }
            let seq = sub.seq.get() + 1;
            sub.seq.set(seq);
            let event = Event {
                event: kind.to_string(),
                seq,
                window_id,
                ts_ms,
                data: data.clone(),
            };
            enqueue(&sub, &event);
        }
        self.subs.borrow_mut().retain(|s| !s.dead.get());
    }
}

/// Register a new subscriber on `conn` (which just delivered a
/// `subscribe` request). Sends the `subscribed` ack (seq 0), then
/// watches the read side so a client close unregisters immediately.
pub fn subscribe(
    daemon: &Rc<crate::Daemon>,
    conn: Rc<ServerConnection>,
    kinds: Option<Vec<String>>,
    window: Option<u64>,
) {
    let ack = Event {
        event: "subscribed".to_string(),
        seq: 0,
        window_id: window,
        ts_ms: now_ms(),
        data: serde_json::json!({ "kinds": kinds }),
    };
    let sub = Rc::new(Subscriber {
        kinds,
        window,
        seq: Cell::new(0),
        conn,
        queue: RefCell::new(VecDeque::new()),
        queued_bytes: Cell::new(0),
        write_in_flight: Cell::new(false),
        dead: Cell::new(false),
    });
    daemon.events.subs.borrow_mut().push(sub.clone());
    enqueue(&sub, &ack);
    watch_disconnect(daemon.clone(), sub);
}

/// Keep an async read pending on the subscriber's connection. A
/// subscriber never sends more requests, so any read completion
/// (EOF, error, or stray bytes followed by close) means the client
/// is going away; drop the subscription immediately rather than
/// discovering it at the next failed write.
///
/// Read with `read_bytes_async`, NOT `read_line_async`: gio-rs
/// 0.20's read_line_async trampoline leaves its length out-param
/// uninitialized when the stream hits EOF (NULL line, no error) and
/// then asserts on the garbage length inside a C callback that
/// cannot unwind, aborting the whole daemon. EOF is this watcher's
/// *normal* completion, so that path is not usable here. The
fn watch_disconnect(daemon: Rc<crate::Daemon>, sub: Rc<Subscriber>) {
    let input = sub.conn.socket.input_stream();
    input.read_bytes_async(
        256,
        glib::Priority::DEFAULT,
        gio::Cancellable::NONE,
        move |res| {
            match res {
                // Stray input from a confused client: ignore it, keep
                // watching for the close.
                Ok(bytes) if !bytes.is_empty() => watch_disconnect(daemon, sub),
                // EOF (empty read) or error: the client is gone.
                _ => drop_subscriber(&daemon, &sub),
            }
        },
    );
}

fn drop_subscriber(daemon: &Rc<crate::Daemon>, sub: &Rc<Subscriber>) {
    sub.dead.set(true);
    let _ = sub.conn.socket.close(gio::Cancellable::NONE);
    daemon
        .events
        .subs
        .borrow_mut()
        .retain(|s| !Rc::ptr_eq(s, sub));
}

/// Queue one serialized event and start the write pump. Over the
/// byte cap the subscriber is killed: backpressure drops the client,
/// never blocks the daemon or grows without bound.
fn enqueue(sub: &Rc<Subscriber>, event: &Event) {
    if sub.dead.get() {
        return;
    }
    let Ok(line) = hwatu_ipc::encode_frame(event, MAX_QUEUED_BYTES) else {
        sub.dead.set(true);
        let _ = sub.conn.socket.close(gio::Cancellable::NONE);
        return;
    };
    let queued = sub.queued_bytes.get() + line.len();
    if queued > MAX_QUEUED_BYTES {
        sub.dead.set(true);
        let _ = sub.conn.socket.close(gio::Cancellable::NONE);
        return;
    }
    sub.queued_bytes.set(queued);
    sub.queue.borrow_mut().push_back(line);
    pump(sub.clone());
}

/// Async write pump: one in-flight write per subscriber, chaining
/// through the queue. A write error marks the subscriber dead; the
/// registry reaps it on the next emit (or the disconnect watcher
/// already did).
fn pump(sub: Rc<Subscriber>) {
    if sub.write_in_flight.get() || sub.dead.get() {
        return;
    }
    let Some(buf) = sub.queue.borrow_mut().pop_front() else {
        return;
    };
    sub.queued_bytes.set(sub.queued_bytes.get() - buf.len());
    sub.write_in_flight.set(true);
    let out = sub.conn.socket.output_stream();
    out.write_all_async(
        buf,
        glib::Priority::DEFAULT,
        gio::Cancellable::NONE,
        move |res| {
            sub.write_in_flight.set(false);
            match res {
                Ok(_) => pump(sub),
                Err(_) => sub.dead.set(true),
            }
        },
    );
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    type Filter = (Option<Vec<String>>, Option<u64>);

    fn sub(kinds: Option<Vec<String>>, window: Option<u64>) -> Filter {
        (kinds, window)
    }

    fn wants((kinds, window): &Filter, kind: &str, window_id: Option<u64>) -> bool {
        // Mirror of Subscriber::wants without a live socket; keep in
        // sync (the logic is three lines on purpose).
        if let Some(w) = window {
            if window_id != Some(*w) {
                return false;
            }
        }
        if let Some(kinds) = kinds {
            if !kinds.iter().any(|k| k == kind) {
                return false;
            }
        }
        true
    }

    /// Filters compose: no filter matches everything, kind filters
    /// match listed kinds only, window filters match that window only,
    /// and every advertised kind is a valid filter value.
    #[test]
    fn subscriber_filters() {
        let all = sub(None, None);
        for kind in hwatu_ipc::EVENT_KINDS {
            assert!(wants(&all, kind, Some(1)));
            assert!(wants(&all, kind, None));
        }
        let loads = sub(Some(vec!["load".into()]), None);
        assert!(wants(&loads, "load", Some(1)));
        assert!(!wants(&loads, "console", Some(1)));
        let win7 = sub(None, Some(7));
        assert!(wants(&win7, "load", Some(7)));
        assert!(!wants(&win7, "load", Some(8)));
        assert!(!wants(&win7, "download", None)); // window-less event, window filter on
        let both = sub(Some(vec!["console".into()]), Some(7));
        assert!(wants(&both, "console", Some(7)));
        assert!(!wants(&both, "console", Some(8)));
        assert!(!wants(&both, "load", Some(7)));
    }
}
