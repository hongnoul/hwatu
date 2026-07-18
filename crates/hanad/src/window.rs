//! Browser window: one WebView per toplevel, zero chrome.
//! The tiling WM is the tab bar.

use crate::Daemon;
use gtk::prelude::*;
use hana_ipc::WindowInfo;
use std::rc::Rc;
use webkit6::prelude::*;

pub struct BrowserWindow {
    pub id: u64,
    pub window: gtk::Window,
    pub webview: webkit6::WebView,
}

/// Build a fully configured WebView. Called for the prewarm pool and as
/// a fallback; all engine knobs live here.
pub fn build_webview() -> webkit6::WebView {
    let view = webkit6::WebView::new();
    if let Some(settings) = webkit6::prelude::WebViewExt::settings(&view) {
        settings.set_enable_developer_extras(true);
        // Render as intended: leave JS, media, canvas, webgl at defaults.
        settings.set_enable_page_cache(true); // bfcache
    }
    view
}

impl BrowserWindow {
    pub fn open(daemon: &Rc<Daemon>, url: Option<String>, app_id: Option<String>) -> WindowInfo {
        let id = daemon.alloc_id();
        let webview = daemon.take_webview();

        let window = gtk::Window::builder()
            .application(&daemon.app)
            .default_width(1024)
            .default_height(768)
            .title("hana-fuda")
            .child(&webview)
            .build();

        // Tiling WMs key window rules off app_id / WM_CLASS. GTK derives
        // it from the application id, so per-window overrides use the
        // startup-id-free path: set the title prefix and let users match
        // on that, plus honor an explicit app_id via a separate
        // GtkApplication is post-MVP. Record it for `hana list` for now.
        let _ = &app_id;

        let target = url.unwrap_or_else(|| "about:blank".into());
        webview.load_uri(&target);

        // Window title follows page title so WMs can display it.
        {
            let win = window.clone();
            webview.connect_title_notify(move |wv| {
                let title = wv.title().unwrap_or_default();
                win.set_title(Some(if title.is_empty() {
                    "hana-fuda"
                } else {
                    title.as_str()
                }));
            });
        }

        // Ctrl+q closes the window; the daemon (and engine) stay warm.
        {
            let ctrl = gtk::EventControllerKey::new();
            let win = window.clone();
            ctrl.connect_key_pressed(move |_, key, _, state| {
                if state.contains(gtk::gdk::ModifierType::CONTROL_MASK)
                    && key == gtk::gdk::Key::q
                {
                    win.close();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            window.add_controller(ctrl);
        }

        // Drop from the registry when the WM closes us.
        {
            let daemon = daemon.clone();
            window.connect_close_request(move |_| {
                daemon.windows.borrow_mut().remove(&id);
                glib::Propagation::Proceed
            });
        }

        window.present();

        let info = WindowInfo {
            id,
            url: target.clone(),
            title: String::new(),
        };
        daemon
            .windows
            .borrow_mut()
            .insert(id, BrowserWindow { id, window, webview });
        info
    }

    pub fn info(&self) -> WindowInfo {
        WindowInfo {
            id: self.id,
            url: self
                .webview
                .uri()
                .map(|u| u.to_string())
                .unwrap_or_default(),
            title: self
                .webview
                .title()
                .map(|t| t.to_string())
                .unwrap_or_default(),
        }
    }
}
