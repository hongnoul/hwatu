// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! Search-engine fallback: bar/CLI input that doesn't look like a URL
//! becomes a web search.
//!
//! The engine lives in `~/.config/hwatu/search.conf` (honoring
//! `XDG_CONFIG_HOME`): a single line that is either a known engine
//! name (`duckduckgo`, `google`, ...) or a full URL template with
//! `%s` for the query:
//!
//! ```text
//! # ~/.config/hwatu/search.conf
//! google
//! ```
//!
//! The installer offers this choice interactively; no file (or an
//! unrecognized line) means DuckDuckGo. The file is read per lookup,
//! so edits apply without a daemon restart.

/// Known engines, name -> URL template. First entry is the default.
pub const ENGINES: &[(&str, &str)] = &[
    ("duckduckgo", "https://duckduckgo.com/?q=%s"),
    ("google", "https://www.google.com/search?q=%s"),
    ("bing", "https://www.bing.com/search?q=%s"),
    ("brave", "https://search.brave.com/search?q=%s"),
    ("startpage", "https://www.startpage.com/sp/search?query=%s"),
    ("kagi", "https://kagi.com/search?q=%s"),
    ("ecosia", "https://www.ecosia.org/search?q=%s"),
];

/// Search URL for `query` using the configured engine.
pub fn url_for(query: &str) -> String {
    template().replace("%s", &encode(query))
}

/// Display label for the active engine: the known-engine name, or the
/// host of a custom template. For the launcher page.
pub fn engine_label() -> String {
    let template = template();
    if let Some((name, _)) = ENGINES.iter().find(|(_, t)| *t == template) {
        return (*name).to_string();
    }
    template
        .split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or("custom")
        .to_string()
}

/// The active URL template: search.conf if valid, else the default.
fn template() -> String {
    std::fs::read_to_string(config_file())
        .ok()
        .and_then(|s| parse_template(&s))
        .unwrap_or_else(|| ENGINES[0].1.to_string())
}

/// First meaningful line of search.conf -> URL template. Accepts an
/// engine name or a custom template containing `%s`.
fn parse_template(conf: &str) -> Option<String> {
    let line = conf
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))?;
    if let Some((_, template)) = ENGINES
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(line))
    {
        return Some(template.to_string());
    }
    if line.contains("%s") && line.contains("://") {
        return Some(line.to_string());
    }
    None
}

fn config_file() -> std::path::PathBuf {
    glib::user_config_dir().join("hwatu").join("search.conf")
}

/// Percent-encode a query string (spaces become `+`, which every
/// engine accepts in the query component).
fn encode(query: &str) -> String {
    let mut out = String::with_capacity(query.len());
    for byte in query.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_engine_names() {
        assert_eq!(
            parse_template("google").as_deref(),
            Some("https://www.google.com/search?q=%s")
        );
        assert_eq!(
            parse_template("  DuckDuckGo  ").as_deref(),
            Some("https://duckduckgo.com/?q=%s")
        );
    }

    #[test]
    fn comments_and_blanks_skipped() {
        assert_eq!(
            parse_template("# my engine\n\nkagi\n").as_deref(),
            Some("https://kagi.com/search?q=%s")
        );
    }

    #[test]
    fn custom_template_passes_through() {
        assert_eq!(
            parse_template("https://example.com/find?q=%s").as_deref(),
            Some("https://example.com/find?q=%s")
        );
    }

    #[test]
    fn junk_is_rejected() {
        assert_eq!(parse_template("yahoo"), None); // unknown name
        assert_eq!(parse_template("https://example.com/"), None); // no %s
        assert_eq!(parse_template("# only comments\n"), None);
        assert_eq!(parse_template(""), None);
    }

    #[test]
    fn encodes_queries() {
        assert_eq!(encode("rust borrow checker"), "rust+borrow+checker");
        assert_eq!(encode("c++ & rust?"), "c%2B%2B+%26+rust%3F");
        assert_eq!(encode("한글"), "%ED%95%9C%EA%B8%80");
    }
}
