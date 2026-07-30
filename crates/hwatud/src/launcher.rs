// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! The launcher page: what a bare `hwatu` shows instead of a blank
//! window. A generated HTML page listing the live keybindings (from
//! the resolved [`crate::keys::Keymap`], so `keys.conf` overrides
//! show up) and the active search engine. Loaded via `load_html`, so
//! no network, no disk, no chrome.

use crate::keys::{Action, Keymap};

/// Actions shown on the page, in display order. Find variants beyond
/// `/` and history keys are summarized rather than exhaustive: the
/// page is a reminder, not a manual.
const SHOWN: &[Action] = &[
    Action::NewWindow,
    Action::UrlOpen,
    Action::UrlEdit,
    Action::Find,
    Action::Back,
    Action::Forward,
    Action::Reload,
    Action::ScrollDown,
    Action::ScrollUp,
    Action::ZoomIn,
    Action::ZoomOut,
    Action::Close,
];

/// The hanafuda boar card from the docs site, inlined at compile time:
/// the launcher must render with no network and no disk.
const BOAR_SVG: &str = include_str!("../assets/boar.svg");

/// Generated HTML for the launcher page.
pub fn html(keymap: &Keymap) -> String {
    let mut rows = String::new();
    for action in SHOWN {
        let chords = keymap.chords_for(*action);
        if chords.is_empty() {
            continue; // unbound via keys.conf
        }
        let keys = chords
            .iter()
            .map(|c| format!("<kbd>{}</kbd>", escape(c)))
            .collect::<Vec<_>>()
            .join(" ");
        rows.push_str(&format!(
            "<tr><td>{keys}</td><td>{}</td></tr>\n",
            action.describe()
        ));
    }
    let engine = escape(&crate::search::engine_label());
    format!(
        r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>hwatu</title><style>
  html {{ height: 100%; }}
  body {{
    margin: 0; min-height: 100%; display: flex;
    align-items: center; justify-content: center;
    background: #181818; color: #d8d8d8;
    font: 13px monospace;
  }}
  main {{ text-align: left; }}
  .card {{ margin: 0 0 14px; }}
  .card svg {{ display: block; height: 96px; width: auto; border-radius: 4px; }}
  table {{ border-collapse: collapse; }}
  td {{ padding: 3px 14px 3px 0; color: #9a9a9a; }}
  td:first-child {{ text-align: right; }}
  kbd {{
    background: #242424; border: 1px solid #333; border-radius: 3px;
    padding: 1px 5px; color: #d8d8d8; font: inherit;
  }}
  p {{ color: #6a6a6a; margin: 14px 0 0; }}
</style></head><body><main>
  <div class="card">{BOAR_SVG}</div>
  <table>
{rows}  </table>
  <p>type a URL or search ({engine}) &mdash; Esc closes this window</p>
</main></body></html>"#
    )
}

/// The launcher's address. Served by the `hwatu://` scheme handler
/// (see [`register_scheme`]), so reloads, discard/restore, and crash
/// recovery all regenerate the page like any other URL.
pub const URI: &str = "hwatu://launcher";

/// Register the `hwatu://` internal scheme on the default WebContext.
/// Call once at daemon startup, before any WebView exists.
pub fn register_scheme(daemon: &std::rc::Rc<crate::Daemon>) {
    use gtk::gio;
    let daemon = daemon.clone();
    webkit6::WebContext::default()
        .expect("default WebContext")
        .register_uri_scheme("hwatu", move |request| {
            let page = match request.uri().as_deref() {
                Some(URI) => html(&daemon.keymap),
                other => format!(
                    "<!DOCTYPE html><meta charset=\"utf-8\">no such page: {}",
                    escape(other.unwrap_or(""))
                ),
            };
            let bytes = glib::Bytes::from_owned(page.into_bytes());
            let stream = gio::MemoryInputStream::from_bytes(&bytes);
            request.finish(&stream, bytes.len() as i64, Some("text/html"));
        });
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_keymap_renders_all_rows() {
        let page = html(&Keymap::default());
        assert!(page.contains("<kbd>o</kbd>"));
        assert!(page.contains("open URL / search"));
        assert!(page.contains("<kbd>ctrl+l</kbd> <kbd>O</kbd>"));
        assert!(page.contains("<kbd>ctrl+J</kbd>")); // ctrl+shift+j, shift folded into case
        assert!(page.contains("<kbd>/</kbd>"));
        assert!(page.contains("close window"));
    }

    #[test]
    fn unbound_actions_are_omitted() {
        let mut map = Keymap::default();
        map.apply_line("close = none").unwrap();
        let page = html(&map);
        assert!(!page.contains("close window"));
    }

    #[test]
    fn escapes_html() {
        assert_eq!(escape("<a&b>"), "&lt;a&amp;b&gt;");
    }
}

#[cfg(test)]
mod preview {
    /// `cargo test -p hwatud preview -- --ignored --nocapture` dumps
    /// the page for visual inspection.
    #[test]
    #[ignore]
    fn dump() {
        let path = std::env::temp_dir().join("hwatu-launcher-preview.html");
        std::fs::write(&path, super::html(&crate::keys::Keymap::default())).unwrap();
        println!("wrote {}", path.display());
    }
}
