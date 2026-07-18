//! The bar: hwatu's single piece of chrome, a one-line vim-style
//! command bar overlaid at the bottom of the window, hidden until
//! summoned.
//!
//! It is a generic prompt widget with modes, not a find widget:
//! find-in-page (`/`, `?`) is the first mode; y/n permission prompts,
//! TLS interstitials, and download status reuse it.

use gtk::prelude::*;

/// What the bar is currently doing. The owner (BrowserWindow) keys its
/// key handling and callbacks off this.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BarMode {
    Hidden,
    /// Incremental find. `backwards` mirrors vim's `?`.
    Find { backwards: bool },
    /// A yes/no question, e.g. permission or TLS prompts. `tag`
    /// identifies the pending request to the owner.
    Confirm { tag: String },
    /// Passive one-shot message (e.g. "saved foo.pdf"), auto-hides.
    Status,
}

#[derive(Clone)]
pub struct Bar {
    pub root: gtk::Box,
    prefix: gtk::Label,
    pub entry: gtk::Entry,
    status: gtk::Label,
    mode: std::rc::Rc<std::cell::RefCell<BarMode>>,
    hide_timer: std::rc::Rc<std::cell::RefCell<Option<glib::SourceId>>>,
}

impl Bar {
    pub fn new() -> Self {
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(4)
            .halign(gtk::Align::Fill)
            .valign(gtk::Align::End)
            .visible(false)
            .css_classes(["hwatu-bar"])
            .build();

        let prefix = gtk::Label::builder().css_classes(["prefix"]).build();
        let entry = gtk::Entry::builder().has_frame(false).hexpand(true).build();
        let status = gtk::Label::builder().css_classes(["status"]).build();

        root.append(&prefix);
        root.append(&entry);
        root.append(&status);

        Bar {
            root,
            prefix,
            entry,
            status,
            mode: std::rc::Rc::new(std::cell::RefCell::new(BarMode::Hidden)),
            hide_timer: Default::default(),
        }
    }

    pub fn mode(&self) -> BarMode {
        self.mode.borrow().clone()
    }

    pub fn is_open(&self) -> bool {
        *self.mode.borrow() != BarMode::Hidden
    }

    /// Open interactive find. Focuses the entry.
    pub fn open_find(&self, backwards: bool) {
        self.cancel_hide_timer();
        self.mode.replace(BarMode::Find { backwards });
        self.prefix.set_label(if backwards { "?" } else { "/" });
        self.entry.set_text("");
        self.entry.set_visible(true);
        self.status.set_label("");
        self.root.set_visible(true);
        self.entry.grab_focus();
    }

    /// Open a yes/no prompt. No entry; the owner answers on y/n keys.
    pub fn open_confirm(&self, tag: &str, question: &str) {
        self.cancel_hide_timer();
        self.mode.replace(BarMode::Confirm { tag: tag.into() });
        self.prefix.set_label(question);
        self.entry.set_visible(false);
        self.status.set_label("[y/n]");
        self.root.set_visible(true);
    }

    /// Show a transient message for `secs` seconds.
    pub fn flash(&self, message: &str, secs: u64) {
        // Never clobber an interactive mode with a passive message.
        if matches!(*self.mode.borrow(), BarMode::Find { .. } | BarMode::Confirm { .. }) {
            return;
        }
        self.cancel_hide_timer();
        self.mode.replace(BarMode::Status);
        self.prefix.set_label(message);
        self.entry.set_visible(false);
        self.status.set_label("");
        self.root.set_visible(true);

        let bar = self.clone();
        let source = glib::timeout_add_local_once(std::time::Duration::from_secs(secs), move || {
            bar.hide_timer.replace(None);
            if *bar.mode.borrow() == BarMode::Status {
                bar.close();
            }
        });
        self.hide_timer.replace(Some(source));
    }

    /// Match counter etc., right-aligned.
    pub fn set_status(&self, text: &str) {
        self.status.set_label(text);
    }

    pub fn close(&self) {
        self.cancel_hide_timer();
        self.mode.replace(BarMode::Hidden);
        self.root.set_visible(false);
    }

    fn cancel_hide_timer(&self) {
        if let Some(source) = self.hide_timer.borrow_mut().take() {
            source.remove();
        }
    }
}

/// One-time CSS for the bar; call at daemon startup.
pub fn install_css() {
    let css = r#"
        .hwatu-bar {
            background-color: rgba(24, 24, 24, 0.92);
            color: #d8d8d8;
            font-family: monospace;
            font-size: 13px;
            padding: 3px 8px;
        }
        .hwatu-bar entry, .hwatu-bar entry text {
            background: none;
            border: none;
            box-shadow: none;
            outline: none;
            color: inherit;
            caret-color: #d8d8d8;
            padding: 0;
            margin: 0;
            min-height: 0;
        }
        .hwatu-bar label.prefix, .hwatu-bar label.status {
            color: #9a9a9a;
        }
    "#;
    let provider = gtk::CssProvider::new();
    provider.load_from_data(css);
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
