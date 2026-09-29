// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Justin Hong
//! EngineBackend: the single seam between hwatud's portable core and a
//! concrete web engine (P2b, docs/architecture-audit.md).
//!
//! Design discipline:
//! - Capability-shaped, not an API mirror: methods say what the core
//!   needs ("pixels of this area", "run this script at document start"),
//!   never how an engine exposes it.
//! - Every method must be implementable by at least two real backends
//!   (WebKitGTK today, WPE-headless later). Anything only GTK can do is
//!   shell vocabulary and must not appear here.
//! - No gtk/gdk/webkit6 types in any signature. Portable types live in
//!   `crate::vocab`.
//!
//! Migration model: modules move off direct `&webkit6::WebView`
//! parameters onto `&dyn EngineView`. During the transition the
//! WebKitGTK implementation (`WebKitView`) is a thin newtype the
//! existing `BrowserWindow` hands out, so callers migrate one at a
//! time while the tree stays green.

#![allow(dead_code)]

use crate::vocab::{LoadPhase, SnapshotArea};

/// When an injected user script runs relative to document parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptTime {
    /// Before any page script (document-start).
    Start,
    /// After the document is parsed (document-end).
    End,
}

/// Which frames an injected script or style reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameScope {
    All,
    TopOnly,
}

/// Opaque handle to a signal connection, so callers can disconnect
/// without holding engine types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignalHandle(pub u64);

/// One engine-backed page view, engine-neutral. The unit the portable
/// core reasons about; windows, tabs, and pools hold these.
pub trait EngineView {
    // -- navigation & document state ------------------------------------
    fn load_uri(&self, uri: &str);
    fn load_html(&self, html: &str, base_uri: Option<&str>);
    fn uri(&self) -> Option<String>;
    fn title(&self) -> Option<String>;
    fn is_loading(&self) -> bool;

    /// Observe load-phase transitions. Callback receives each phase
    /// change until disconnected.
    fn on_load_phase(&self, cb: Box<dyn Fn(LoadPhase) + 'static>) -> SignalHandle;
    fn disconnect(&self, handle: SignalHandle);

    // -- script execution -----------------------------------------------
    /// Evaluate JS in the page world; callback gets a JSON-serialized
    /// result or an error string.
    fn eval_js(&self, script: &str, cb: Box<dyn FnOnce(Result<String, String>) + 'static>);

    // -- injected content ------------------------------------------------
    /// Install a user script that runs on every subsequent load.
    fn add_user_script(&self, source: &str, time: ScriptTime, frames: FrameScope);

    /// Register a named message channel page JS can post to
    /// (`window.webkit.messageHandlers[name]` today; the trait only
    /// promises "page can send strings to this name").
    fn register_message_handler(&self, name: &str);

    /// Receive messages posted to a registered channel as raw strings.
    fn on_message(&self, name: &str, cb: Box<dyn Fn(String) + 'static>) -> SignalHandle;

    // -- output ----------------------------------------------------------
    /// Rasterize the page. Callback gets PNG bytes.
    fn snapshot_png(
        &self,
        area: SnapshotArea,
        cb: Box<dyn FnOnce(Result<Vec<u8>, String>) + 'static>,
    );
}

/// WebKitGTK implementation of [`EngineView`].
pub struct WebKitView(pub webkit6::WebView);

mod webkitgtk {
    use super::*;
    use glib::translate::ToGlibPtr;
    use gtk::glib;
    use webkit6::prelude::*;

    fn ucm(view: &webkit6::WebView) -> Option<webkit6::UserContentManager> {
        view.user_content_manager()
    }

    impl EngineView for WebKitView {
        fn load_uri(&self, uri: &str) {
            self.0.load_uri(uri);
        }

        fn load_html(&self, html: &str, base_uri: Option<&str>) {
            self.0.load_html(html, base_uri);
        }

        fn uri(&self) -> Option<String> {
            self.0.uri().map(|u| u.to_string())
        }

        fn title(&self) -> Option<String> {
            self.0.title().map(|t| t.to_string())
        }

        fn is_loading(&self) -> bool {
            self.0.is_loading()
        }

        fn on_load_phase(&self, cb: Box<dyn Fn(LoadPhase) + 'static>) -> SignalHandle {
            let id = self
                .0
                .connect_load_changed(move |_, event| cb(LoadPhase::from(event)));
            SignalHandle(unsafe { id.as_raw() })
        }

        fn disconnect(&self, handle: SignalHandle) {
            let obj: &glib::Object = self.0.upcast_ref();
            unsafe {
                glib::gobject_ffi::g_signal_handler_disconnect(
                    obj.to_glib_none().0,
                    handle.0,
                );
            }
        }

        fn eval_js(&self, script: &str, cb: Box<dyn FnOnce(Result<String, String>) + 'static>) {
            self.0.evaluate_javascript(
                script,
                None,
                None,
                gtk::gio::Cancellable::NONE,
                move |result| {
                    cb(match result {
                        Ok(value) => Ok(value.to_json(0).map(|s| s.to_string()).unwrap_or_default()),
                        Err(e) => Err(e.to_string()),
                    })
                },
            );
        }

        fn add_user_script(&self, source: &str, time: ScriptTime, frames: FrameScope) {
            let Some(ucm) = ucm(&self.0) else { return };
            let script = webkit6::UserScript::new(
                source,
                match frames {
                    FrameScope::All => webkit6::UserContentInjectedFrames::AllFrames,
                    FrameScope::TopOnly => webkit6::UserContentInjectedFrames::TopFrame,
                },
                match time {
                    ScriptTime::Start => webkit6::UserScriptInjectionTime::Start,
                    ScriptTime::End => webkit6::UserScriptInjectionTime::End,
                },
                &[],
                &[],
            );
            ucm.add_script(&script);
        }

        fn register_message_handler(&self, name: &str) {
            if let Some(ucm) = ucm(&self.0) {
                ucm.register_script_message_handler(name, None);
            }
        }

        fn on_message(&self, name: &str, cb: Box<dyn Fn(String) + 'static>) -> SignalHandle {
            let Some(ucm) = ucm(&self.0) else {
                return SignalHandle(0);
            };
            let id = ucm.connect_script_message_received(Some(name), move |_, value| {
                cb(value.to_str().to_string());
            });
            SignalHandle(unsafe { id.as_raw() })
        }

        fn snapshot_png(
            &self,
            area: SnapshotArea,
            cb: Box<dyn FnOnce(Result<Vec<u8>, String>) + 'static>,
        ) {
            self.0.snapshot(
                area.to_webkit(),
                webkit6::SnapshotOptions::NONE,
                gtk::gio::Cancellable::NONE,
                move |result| match result {
                    Ok(texture) => {
                        let bytes = texture.save_to_png_bytes();
                        cb(Ok(bytes.to_vec()));
                    }
                    Err(e) => cb(Err(e.to_string())),
                },
            );
        }
    }
}
