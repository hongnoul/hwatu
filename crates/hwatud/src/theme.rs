// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! Theme continuity (roadmap H37): follow the desktop's color scheme
//! so pages' `prefers-color-scheme` resolves like every native app's.
//!
//! GTK4 apps normally get this from libadwaita; hwatud deliberately
//! links no adwaita, so it reads the XDG settings portal directly:
//! `org.freedesktop.portal.Settings` / namespace
//! `org.freedesktop.appearance` / key `color-scheme` (1 = prefer
//! dark), sets `gtk-application-prefer-dark-theme` accordingly —
//! which is exactly the signal WebKitGTK maps into
//! `prefers-color-scheme: dark` — and subscribes to `SettingChanged`
//! so a darkman/nightlight flip mid-session propagates live without
//! a restart. No portal (bare compositor, display-free CI) is fine:
//! the default light scheme stands.

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;

const PORTAL_BUS: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const PORTAL_IFACE: &str = "org.freedesktop.portal.Settings";
const NS: &str = "org.freedesktop.appearance";
const KEY: &str = "color-scheme";

/// Apply `prefer_dark` to GTK (and thereby WebKit's
/// prefers-color-scheme for every WebView).
fn apply(prefer_dark: bool) {
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_application_prefer_dark_theme(prefer_dark);
    }
}

/// Portal value -> prefer-dark. Per the spec: 0 no preference,
/// 1 prefer dark, 2 prefer light; unknown values = no preference.
pub(crate) fn prefers_dark(value: u32) -> bool {
    value == 1
}

/// Start following the desktop color scheme. Call once at startup
/// (after GTK init). Fails soft everywhere: no bus, no portal, or a
/// portal without the appearance namespace all leave the default.
pub fn follow_system() {
    let Ok(conn) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else {
        return;
    };
    // Initial read (async; startup must not block on a portal).
    conn.call(
        Some(PORTAL_BUS),
        PORTAL_PATH,
        PORTAL_IFACE,
        "ReadOne",
        Some(&(NS, KEY).to_variant()),
        Some(glib::VariantTy::new("(v)").unwrap()),
        gio::DBusCallFlags::NONE,
        2000,
        gio::Cancellable::NONE,
        |result| {
            let Ok(reply) = result else { return };
            if let Some(value) = reply
                .get::<(glib::Variant,)>()
                .and_then(|(v,)| v.get::<u32>())
            {
                apply(prefers_dark(value));
            }
        },
    );
    // Live updates.
    conn.signal_subscribe(
        Some(PORTAL_BUS),
        Some(PORTAL_IFACE),
        Some("SettingChanged"),
        Some(PORTAL_PATH),
        None,
        gio::DBusSignalFlags::NONE,
        |_, _, _, _, _, params| {
            let Some((ns, key, value)) = params.get::<(String, String, glib::Variant)>() else {
                return;
            };
            if ns == NS && key == KEY {
                if let Some(value) = value.get::<u32>() {
                    apply(prefers_dark(value));
                }
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Portal spec: only 1 means prefer-dark. 0 (no preference) and
    /// 2 (prefer light) must not darken, nor should garbage values.
    #[test]
    fn portal_value_mapping() {
        assert!(!prefers_dark(0));
        assert!(prefers_dark(1));
        assert!(!prefers_dark(2));
        assert!(!prefers_dark(99));
    }
}
