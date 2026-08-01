// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! The launcher page: what a bare `hwatu` shows instead of a blank
//! window. A single hanafuda card, dealt in deck order: each new
//! launcher window gets the next card (January through December,
//! row by row), wrapping after all 48. Loaded via `load_html`, so
//! no network, no disk, no chrome.

/// The full hanafuda deck, inlined at compile time: the launcher must
/// render with no network and no disk. Deal order is row-major over
/// the deck laid out as 12 month columns x 4 rows (row 1 = each
/// month's highest card), so the 13th deal is January's second card.
/// Art by Louie Mantia, CC BY-SA 4.0 (see assets/cards/ATTRIBUTION.md).
const CARDS: [&str; DECK_SIZE] = [
    include_str!("../assets/cards/00-january-hikari.svg"),
    include_str!("../assets/cards/01-february-tane.svg"),
    include_str!("../assets/cards/02-march-hikari.svg"),
    include_str!("../assets/cards/03-april-tane.svg"),
    include_str!("../assets/cards/04-may-tane.svg"),
    include_str!("../assets/cards/05-june-tane.svg"),
    include_str!("../assets/cards/06-july-tane.svg"),
    include_str!("../assets/cards/07-august-hikari.svg"),
    include_str!("../assets/cards/08-september-tane.svg"),
    include_str!("../assets/cards/09-october-tane.svg"),
    include_str!("../assets/cards/10-november-hikari.svg"),
    include_str!("../assets/cards/11-december-hikari.svg"),
    include_str!("../assets/cards/12-january-tanzaku.svg"),
    include_str!("../assets/cards/13-february-tanzaku.svg"),
    include_str!("../assets/cards/14-march-tanzaku.svg"),
    include_str!("../assets/cards/15-april-tanzaku.svg"),
    include_str!("../assets/cards/16-may-tanzaku.svg"),
    include_str!("../assets/cards/17-june-tanzaku.svg"),
    include_str!("../assets/cards/18-july-tanzaku.svg"),
    include_str!("../assets/cards/19-august-tane.svg"),
    include_str!("../assets/cards/20-september-tanzaku.svg"),
    include_str!("../assets/cards/21-october-tanzaku.svg"),
    include_str!("../assets/cards/22-november-tane.svg"),
    include_str!("../assets/cards/23-december-kasu-1.svg"),
    include_str!("../assets/cards/24-january-kasu-1.svg"),
    include_str!("../assets/cards/25-february-kasu-1.svg"),
    include_str!("../assets/cards/26-march-kasu-1.svg"),
    include_str!("../assets/cards/27-april-kasu-1.svg"),
    include_str!("../assets/cards/28-may-kasu-1.svg"),
    include_str!("../assets/cards/29-june-kasu-1.svg"),
    include_str!("../assets/cards/30-july-kasu-1.svg"),
    include_str!("../assets/cards/31-august-kasu-1.svg"),
    include_str!("../assets/cards/32-september-kasu-1.svg"),
    include_str!("../assets/cards/33-october-kasu-1.svg"),
    include_str!("../assets/cards/34-november-tanzaku.svg"),
    include_str!("../assets/cards/35-december-kasu-2.svg"),
    include_str!("../assets/cards/36-january-kasu-2.svg"),
    include_str!("../assets/cards/37-february-kasu-2.svg"),
    include_str!("../assets/cards/38-march-kasu-2.svg"),
    include_str!("../assets/cards/39-april-kasu-2.svg"),
    include_str!("../assets/cards/40-may-kasu-2.svg"),
    include_str!("../assets/cards/41-june-kasu-2.svg"),
    include_str!("../assets/cards/42-july-kasu-2.svg"),
    include_str!("../assets/cards/43-august-kasu-2.svg"),
    include_str!("../assets/cards/44-september-kasu-2.svg"),
    include_str!("../assets/cards/45-october-kasu-2.svg"),
    include_str!("../assets/cards/46-november-kasu.svg"),
    include_str!("../assets/cards/47-december-kasu-3.svg"),
];

/// Cards in the deck. Deals wrap modulo this.
pub const DECK_SIZE: usize = 48;

/// Generated HTML for the launcher page: one card, centered, nothing
/// else. `deal` indexes [`CARDS`] (wrapped, so any usize is safe).
pub fn html(deal: usize) -> String {
    let card = CARDS[deal % DECK_SIZE];
    format!(
        r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>hwatu</title><style>
  html, body {{ height: 100%; }}
  /* Keep the page backing black, then put alpha on a sibling layer so
     WebKit does not propagate body background to the canvas. */
  html {{ background: #000; }}
  body {{
    margin: 0; display: flex; overflow: hidden;
    align-items: center; justify-content: center;
    background: transparent;
  }}
  .backdrop {{
    position: fixed; inset: 0; z-index: 0; pointer-events: none;
    background: rgba(0, 0, 0, 0.9);
  }}
  /* Fill the pane edge to edge; the SVG viewBox letterboxes itself
     (xMidYMid meet), keeping the card's aspect ratio. */
  .card svg {{
    display: block;
    width: 100vw;
    height: 100vh;
  }}
  .card {{ position: relative; z-index: 1; }}
  /* The card art is portrait. In a landscape pane, lay it on its
     side so it covers the pane instead of a thin centered strip.
     Swap the box to the rotated frame; the layout-box overflow is
     clipped, the visual box is exactly the pane. */
  @media (orientation: landscape) {{
    .card svg {{
      width: 100vh;
      height: 100vw;
      transform: rotate(90deg);
    }}
  }}
</style></head><body>
  <div class="backdrop" aria-hidden="true"></div>
  <div class="card">{card}</div>
</body></html>"#
    )
}

/// The launcher's base address. Served by the `hwatu://` scheme
/// handler (see [`register_scheme`]), so reloads, discard/restore,
/// and crash recovery all regenerate the page like any other URL.
pub const URI: &str = "hwatu://launcher";

/// Address of a specific deal: the card index rides in the URL so a
/// window keeps its card across reload and session restore.
pub fn deal_uri(deal: usize) -> String {
    format!("{URI}?card={}", deal % DECK_SIZE)
}

/// Whether `url` is the launcher page (any deal).
pub fn is_launcher(url: &str) -> bool {
    url == URI || url.starts_with("hwatu://launcher?")
}

/// Parse the deal index out of a launcher URL. Bare [`URI`] and
/// malformed queries fall back to deal 0.
fn deal_from_uri(uri: &str) -> usize {
    uri.split_once("?card=")
        .and_then(|(_, n)| n.parse::<usize>().ok())
        .unwrap_or(0)
        % DECK_SIZE
}

/// Register the `hwatu://` internal scheme on the default WebContext.
/// Call once at daemon startup, before any WebView exists.
pub fn register_scheme() {
    use gtk::gio;
    webkit6::WebContext::default()
        .expect("default WebContext")
        .register_uri_scheme("hwatu", move |request| {
            let page = match request.uri().as_deref() {
                Some(uri) if is_launcher(uri) => html(deal_from_uri(uri)),
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
    fn every_card_renders() {
        for deal in 0..DECK_SIZE {
            let page = html(deal);
            assert!(page.contains("<svg"), "deal {deal} has no svg");
            // Card-only page: no keybind table, no footer hint.
            assert!(!page.contains("<kbd>"));
            assert!(!page.contains("<table"));
        }
    }

    #[test]
    fn deals_are_distinct_and_wrap() {
        assert_ne!(html(0), html(1));
        assert_eq!(html(0), html(DECK_SIZE));
        assert_eq!(html(12), html(12 + DECK_SIZE));
    }

    #[test]
    fn pane_background_is_translucent_without_card_opacity() {
        let page = html(0);
        assert!(page.contains("html { background: #000; }"));
        assert!(page.contains("background: rgba(0, 0, 0, 0.9);"));
        assert!(!page.contains("opacity:"));
    }

    #[test]
    fn thirteenth_deal_is_january_row_two() {
        // Deals 0-11 are January..December row one; deal 12 (the 13th
        // window) starts row two back at January. Different card art,
        // same month: both January cards share no art with December.
        assert_ne!(CARDS[12], CARDS[0]);
        assert_ne!(CARDS[12], CARDS[11]);
        // Wrap: deal 48 re-deals card 0.
        assert_eq!(html(48), html(0));
    }

    #[test]
    fn deal_uri_roundtrip() {
        for deal in [0, 1, 12, 47] {
            assert_eq!(deal_from_uri(&deal_uri(deal)), deal);
        }
        assert_eq!(deal_from_uri(&deal_uri(48)), 0);
        assert_eq!(deal_from_uri(URI), 0);
        assert_eq!(deal_from_uri("hwatu://launcher?card=junk"), 0);
    }

    #[test]
    fn launcher_urls_are_recognized() {
        assert!(is_launcher(URI));
        assert!(is_launcher(&deal_uri(12)));
        assert!(!is_launcher("hwatu://other"));
        assert!(!is_launcher("https://example.com"));
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
        std::fs::write(&path, super::html(0)).unwrap();
        println!("wrote {}", path.display());
    }
}
