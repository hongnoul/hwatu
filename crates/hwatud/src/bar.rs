// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! The prompt surface: hwatu's single piece of chrome, a small
//! monochrome command surface overlaid near the top of the window,
//! hidden until summoned.
//!
//! It is a generic prompt widget with modes, not a find widget:
//! find-in-page is the first mode; yes/no permission prompts,
//! TLS interstitials, and download status reuse it.

use gtk::prelude::*;

/// What the bar is currently doing. The owner (BrowserWindow) keys its
/// key handling and callbacks off this.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BarMode {
    Hidden,
    /// Incremental find, optionally searching backwards.
    Find {
        backwards: bool,
    },
    /// URL entry: Enter navigates the window, Esc cancels.
    Url,
    /// Command palette: fuzzy action search. Enter runs the selected
    /// action, Up/Down move, Esc cancels.
    Palette,
    /// A yes/no question, e.g. permission or TLS prompts. `tag`
    /// identifies the pending request to the owner.
    Confirm {
        tag: String,
    },
    /// Passive one-shot message (e.g. "saved foo.pdf"), auto-hides.
    Status,
}

#[derive(Clone)]
pub struct Bar {
    pub root: gtk::Box,
    /// Palette results, stacked above the input row; hidden outside
    /// [`BarMode::Palette`].
    list: gtk::Box,
    prefix: gtk::Label,
    pub entry: gtk::Entry,
    status: gtk::Label,
    mode: std::rc::Rc<std::cell::RefCell<BarMode>>,
    hide_timer: std::rc::Rc<std::cell::RefCell<Option<glib::SourceId>>>,
}

impl Bar {
    pub fn new() -> Self {
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .halign(gtk::Align::Fill)
            .valign(gtk::Align::Start)
            .hexpand(true)
            .margin_top(24)
            .margin_start(0)
            .margin_end(0)
            .visible(false)
            .css_classes(["hwatu-bar"])
            .build();

        let list = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .visible(false)
            .css_classes(["palette-list"])
            .build();

        let row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(4)
            .build();

        let prefix = gtk::Label::builder().css_classes(["prefix"]).build();
        let entry = gtk::Entry::builder()
            .has_frame(false)
            .hexpand(true)
            .xalign(0.5)
            .build();
        let status = gtk::Label::builder().css_classes(["status"]).build();

        row.append(&prefix);
        row.append(&entry);
        row.append(&status);
        root.append(&list);
        root.append(&row);

        Bar {
            root,
            list,
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
        self.hide_list();
        self.mode.replace(BarMode::Find { backwards });
        self.prefix
            .set_label(if backwards { "Find backwards" } else { "Find" });
        self.entry.set_text("");
        self.entry.set_visible(true);
        self.status.set_label("");
        self.root.set_visible(true);
        self.entry.grab_focus();
    }

    /// Open the URL prompt, prefilled (and selected) with `current` so
    /// typing replaces it and editing is one keystroke away.
    pub fn open_url(&self, current: &str) {
        self.cancel_hide_timer();
        self.hide_list();
        self.mode.replace(BarMode::Url);
        self.prefix.set_label("Open");
        self.entry.set_text(current);
        self.entry.set_visible(true);
        self.status.set_label("");
        self.root.set_visible(true);
        self.entry.grab_focus();
        self.entry.select_region(0, -1);
    }

    /// Open the command palette. The owner populates results via
    /// [`Self::set_palette_rows`] on every entry change.
    pub fn open_palette(&self) {
        self.cancel_hide_timer();
        self.mode.replace(BarMode::Palette);
        self.prefix.set_label("Command");
        self.entry.set_text("");
        self.entry.set_visible(true);
        self.status.set_label("");
        self.list.set_visible(true);
        self.root.set_visible(true);
        self.entry.grab_focus();
    }

    /// Replace the palette rows. Each row is (title, chord hint);
    /// `selected` gets the highlight. Rows render top-to-bottom in
    /// rank order; the caller caps the list length.
    pub fn set_palette_rows(&self, rows: &[(String, String)], selected: usize) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        for (i, (title, detail)) in rows.iter().enumerate() {
            let row = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(8)
                .css_classes(["palette-row"])
                .build();
            if i == selected {
                row.add_css_class("selected");
            }
            let title = gtk::Label::builder()
                .label(title)
                .halign(gtk::Align::Start)
                .hexpand(true)
                .build();
            let detail = gtk::Label::builder()
                .label(detail)
                .halign(gtk::Align::End)
                .css_classes(["status"])
                .build();
            row.append(&title);
            row.append(&detail);
            self.list.append(&row);
        }
        self.status
            .set_label(if rows.is_empty() { "no match" } else { "" });
    }

    /// Move the highlight without rebuilding rows.
    pub fn set_palette_selected(&self, selected: usize) {
        let mut child = self.list.first_child();
        let mut i = 0usize;
        while let Some(row) = child {
            if i == selected {
                row.add_css_class("selected");
            } else {
                row.remove_css_class("selected");
            }
            child = row.next_sibling();
            i += 1;
        }
    }

    /// Open a yes/no prompt. No entry; the owner answers on y/n keys.
    pub fn open_confirm(&self, tag: &str, question: &str) {
        self.cancel_hide_timer();
        self.hide_list();
        self.mode.replace(BarMode::Confirm { tag: tag.into() });
        self.prefix.set_label(question);
        self.entry.set_visible(false);
        self.status.set_label("yes / no");
        self.root.set_visible(true);
    }

    /// Show a transient message for `secs` seconds.
    pub fn flash(&self, message: &str, secs: u64) {
        // Never clobber an interactive mode with a passive message.
        if matches!(
            *self.mode.borrow(),
            BarMode::Find { .. } | BarMode::Url | BarMode::Palette | BarMode::Confirm { .. }
        ) {
            return;
        }
        self.cancel_hide_timer();
        self.hide_list();
        self.mode.replace(BarMode::Status);
        self.prefix.set_label(message);
        self.entry.set_visible(false);
        self.status.set_label("");
        self.root.set_visible(true);

        let bar = self.clone();
        let source =
            glib::timeout_add_local_once(std::time::Duration::from_secs(secs), move || {
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
        self.hide_list();
        self.mode.replace(BarMode::Hidden);
        self.root.set_visible(false);
    }

    fn hide_list(&self) {
        self.list.set_visible(false);
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
    }

    fn cancel_hide_timer(&self) {
        if let Some(source) = self.hide_timer.borrow_mut().take() {
            source.remove();
        }
    }
}

/// One-time CSS for the prompt surface; call at daemon startup.
pub fn install_css() {
    let css = r#"
        .hwatu-bar {
            background-color: rgba(24, 24, 24, 0.62);
            color: #ffffff;
            font-family: "Inter", "Roboto", "Noto Sans", "Cantarell", sans-serif;
            font-size: 13px;
            padding: 10px 16px;
            border: none;
            border-radius: 0;
            box-shadow: 0 4px 18px rgba(0, 0, 0, 0.25);
        }
        .hwatu-bar entry, .hwatu-bar entry text {
            background: none;
            border: none;
            box-shadow: none;
            outline: none;
            color: inherit;
            caret-color: #ffffff;
            padding: 0;
            margin: 0;
            min-height: 0;
        }
        .hwatu-bar label.prefix, .hwatu-bar label.status {
            color: rgba(255, 255, 255, 0.65);
        }
        .hwatu-bar .palette-list {
            padding: 4px 0 8px;
        }
        .hwatu-bar .palette-row {
            padding: 5px 8px;
            border-radius: 0;
        }
        .hwatu-bar .palette-row.selected {
            background-color: rgba(255, 255, 255, 0.16);
            color: #ffffff;
        }
        .hwatu-recovery {
            background-color: rgba(18, 18, 18, 0.88);
            color: #e8e8e8;
            border-radius: 12px;
            padding: 20px 24px;
            margin: 24px;
            box-shadow: 0 8px 32px rgba(0, 0, 0, 0.35);
        }
        .hwatu-recovery label.title {
            font-size: 20px;
            font-weight: 700;
        }
        .hwatu-recovery label.detail {
            color: #d0d0d0;
        }
        .hwatu-recovery label.hint {
            color: #9a9a9a;
            font-family: monospace;
            font-size: 12px;
        }
    "#;
    let provider = gtk::CssProvider::new();
    provider.load_from_string(css);
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
