// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! Web notifications (roadmap H4): forward WebKit's `show-notification`
//! to the desktop over D-Bus (`org.freedesktop.Notifications`), and
//! route notification clicks back to window focus.
//!
//! WebKitGTK only shows notifications itself when built against
//! libnotify, which distro packages often are not; without this
//! forwarder a granted permission is silent — worse than a denied one,
//! because the site believes it is notifying. Direct D-Bus keeps the
//! dependency surface at gio, which hwatud already links.

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use webkit6::prelude::*;

/// Sent notification ids (desktop id -> WebKit notification), so a
/// desktop click can be routed to `webkit_notification_clicked()` and
/// a page-side `close()` can retract the desktop popup.
type Live = Rc<RefCell<HashMap<u32, webkit6::Notification>>>;

thread_local! {
    static LIVE: Live = Rc::new(RefCell::new(HashMap::new()));
    /// One signal subscription for ActionInvoked/NotificationClosed.
    static SUBSCRIBED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Wire one window's WebView. Returns notification clicks to `on_click`
/// (used to present the window: a clicked notification is an explicit
/// user request for that page).
pub fn wire_view(webview: &webkit6::WebView, on_click: impl Fn() + 'static) {
    let on_click = Rc::new(on_click);
    webview.connect_show_notification(move |_, notification| {
        let Some(conn) = session_bus() else {
            eprintln!("hwatud: no session bus; web notification dropped");
            return true; // handled: don't fall into WebKit's libnotify path
        };
        ensure_click_subscription(&conn);
        send(&conn, notification, on_click.clone());
        true
    });
}

fn session_bus() -> Option<gio::DBusConnection> {
    gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).ok()
}

/// Forward one notification. Async: the reply carries the desktop id
/// needed to correlate clicks; failures degrade to a log line.
fn send(conn: &gio::DBusConnection, notification: &webkit6::Notification, on_click: Rc<dyn Fn()>) {
    let title = notification.title().unwrap_or_default().to_string();
    let body = notification.body().unwrap_or_default().to_string();
    // "default" is the freedesktop action for "the notification body
    // itself was activated" — servers that support clicks need it
    // listed to emit ActionInvoked.
    let actions = ["default", "Open"];
    let hints: HashMap<&str, glib::Variant> = HashMap::new();
    let args = (
        "hwatu",       // app_name
        0u32,          // replaces_id (0 = new)
        "web-browser", // themed icon
        title,
        body,
        &actions[..],
        hints,
        -1i32, // server-default expiry
    )
        .to_variant();
    let notification = notification.clone();
    conn.call(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
        "Notify",
        Some(&args),
        Some(glib::VariantTy::new("(u)").unwrap()),
        gio::DBusCallFlags::NONE,
        2000,
        gio::Cancellable::NONE,
        move |result| match result {
            Ok(reply) => {
                let (id,): (u32,) = reply.get().unwrap_or((0,));
                if id == 0 {
                    return;
                }
                LIVE.with(|live| live.borrow_mut().insert(id, notification.clone()));
                // Page-side close() retracts the desktop popup.
                let conn2 = session_bus();
                notification.connect_closed(move |_| {
                    LIVE.with(|live| {
                        live.borrow_mut().remove(&id);
                    });
                    if let Some(conn) = &conn2 {
                        conn.call(
                            Some("org.freedesktop.Notifications"),
                            "/org/freedesktop/Notifications",
                            "org.freedesktop.Notifications",
                            "CloseNotification",
                            Some(&(id,).to_variant()),
                            None,
                            gio::DBusCallFlags::NONE,
                            2000,
                            gio::Cancellable::NONE,
                            |_| {},
                        );
                    }
                });
                // Desktop click -> page `click` event + window focus.
                let on_click = on_click.clone();
                CLICK_HANDLERS.with(|handlers| {
                    handlers.borrow_mut().insert(id, on_click);
                });
            }
            Err(error) => {
                eprintln!("hwatud: web notification failed: {error}");
            }
        },
    );
}

thread_local! {
    static CLICK_HANDLERS: RefCell<HashMap<u32, Rc<dyn Fn()>>> = RefCell::new(HashMap::new());
}

/// Subscribe once to the server's ActionInvoked/NotificationClosed
/// signals; routes clicks to the WebKit notification (fires the page's
/// `notification.onclick`) and the window-focus callback.
fn ensure_click_subscription(conn: &gio::DBusConnection) {
    if SUBSCRIBED.with(|s| s.replace(true)) {
        return;
    }
    conn.signal_subscribe(
        Some("org.freedesktop.Notifications"),
        Some("org.freedesktop.Notifications"),
        Some("ActionInvoked"),
        Some("/org/freedesktop/Notifications"),
        None,
        gio::DBusSignalFlags::NONE,
        |_, _, _, _, _, params| {
            let Some((id, _action)) = params.get::<(u32, String)>() else {
                return;
            };
            let notification = LIVE.with(|live| live.borrow().get(&id).cloned());
            if let Some(notification) = notification {
                notification.clicked();
            }
            let handler = CLICK_HANDLERS.with(|handlers| handlers.borrow().get(&id).cloned());
            if let Some(handler) = handler {
                handler();
            }
        },
    );
    conn.signal_subscribe(
        Some("org.freedesktop.Notifications"),
        Some("org.freedesktop.Notifications"),
        Some("NotificationClosed"),
        Some("/org/freedesktop/Notifications"),
        None,
        gio::DBusSignalFlags::NONE,
        |_, _, _, _, _, params| {
            let Some((id, _reason)) = params.get::<(u32, u32)>() else {
                return;
            };
            LIVE.with(|live| live.borrow_mut().remove(&id));
            CLICK_HANDLERS.with(|handlers| handlers.borrow_mut().remove(&id));
        },
    );
}
