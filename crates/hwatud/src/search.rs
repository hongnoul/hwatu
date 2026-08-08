// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! Search-engine fallback: bar/CLI input that doesn't look like a URL
//! becomes a web search. Plus (roadmap H13) per-engine search
//! keywords and named quickmarks.
//!
//! The engine lives in `~/.config/hwatu/search.conf` (honoring
//! `XDG_CONFIG_HOME`). The first meaningful line is the default
//! engine: a known engine name (`duckduckgo`, `google`, ...) or a
//! full URL template with `%s` for the query. Further lines define
//! search keywords: `<keyword> <engine-name-or-template>`:
//!
//! ```text
//! # ~/.config/hwatu/search.conf
//! google
//! w https://en.wikipedia.org/w/index.php?search=%s
//! gh https://github.com/search?q=%s
//! d duckduckgo
//! ```
//!
//! `w foo bar` then searches Wikipedia for "foo bar". Keywords only
//! fire on the first whitespace-separated token, and only when input
//! isn't already a URL.
//!
//! Quickmarks live in `~/.config/hwatu/quickmarks.conf`, one
//! `<name> <url>` per line; typing exactly `<name>` in the bar/CLI
//! goes straight to the URL:
//!
//! ```text
//! # ~/.config/hwatu/quickmarks.conf
//! news https://news.ycombinator.com/
//! mail https://mail.proton.me/
//! ```
//!
//! Both files are read per lookup, so edits apply without a daemon
//! restart. The installer offers the engine choice interactively; no
//! file (or an unrecognized line) means DuckDuckGo.

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

/// Search URL for `query` using the configured engine. Checks
/// quickmarks and search keywords first (roadmap H13): an exact
/// quickmark name wins, then a leading keyword, then the default
/// engine.
pub fn url_for(query: &str) -> String {
    let query = query.trim();
    if let Some(url) = quickmark(query) {
        return url;
    }
    if let Some((keyword, rest)) = query.split_once(char::is_whitespace) {
        if let Some(template) = keyword_template(keyword) {
            return template.replace("%s", &encode(rest.trim()));
        }
    }
    template().replace("%s", &encode(query))
}

/// The active URL template: search.conf if valid, else the default.
fn template() -> String {
    std::fs::read_to_string(config_file())
        .ok()
        .and_then(|s| parse_template(&s))
        .unwrap_or_else(|| ENGINES[0].1.to_string())
}

/// First meaningful line of search.conf -> URL template. Accepts an
/// engine name or a custom template containing `%s`. Keyword lines
/// (`<keyword> <spec>`, containing whitespace) never parse as the
/// default engine, so a misordered file degrades to the default
/// rather than producing a broken template.
fn parse_template(conf: &str) -> Option<String> {
    let line = conf
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))?;
    if line.contains(char::is_whitespace) {
        return None;
    }
    resolve_engine(line)
}

/// An engine name or a raw `%s` template -> URL template.
fn resolve_engine(spec: &str) -> Option<String> {
    if let Some((_, template)) = ENGINES
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(spec))
    {
        return Some(template.to_string());
    }
    if spec.contains("%s") && spec.contains("://") {
        return Some(spec.to_string());
    }
    None
}

/// Template for a search keyword, from search.conf lines 2+ of the
/// form `<keyword> <engine-name-or-template>` (roadmap H13).
fn keyword_template(keyword: &str) -> Option<String> {
    let conf = std::fs::read_to_string(config_file()).ok()?;
    parse_keyword(&conf, keyword)
}

fn parse_keyword(conf: &str, keyword: &str) -> Option<String> {
    for line in conf.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((name, spec)) = line.split_once(char::is_whitespace) else {
            continue; // the default-engine line
        };
        if name.eq_ignore_ascii_case(keyword) {
            return resolve_engine(spec.trim());
        }
    }
    None
}

/// Quickmark URL for exactly-`name` input (roadmap H13).
fn quickmark(name: &str) -> Option<String> {
    if name.is_empty() || name.contains(char::is_whitespace) {
        return None;
    }
    let conf = std::fs::read_to_string(quickmarks_file()).ok()?;
    parse_quickmark(&conf, name)
}

fn parse_quickmark(conf: &str, name: &str) -> Option<String> {
    for line in conf.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((mark, url)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let url = url.trim();
        if mark.eq_ignore_ascii_case(name) && url.contains("://") {
            return Some(url.to_string());
        }
    }
    None
}

fn config_file() -> std::path::PathBuf {
    glib::user_config_dir().join("hwatu").join("search.conf")
}

fn quickmarks_file() -> std::path::PathBuf {
    glib::user_config_dir()
        .join("hwatu")
        .join("quickmarks.conf")
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

    #[test]
    fn keywords_resolve_names_and_templates() {
        let conf = "google\nw https://en.wikipedia.org/w/index.php?search=%s\nd duckduckgo\n";
        assert_eq!(
            parse_keyword(conf, "w").as_deref(),
            Some("https://en.wikipedia.org/w/index.php?search=%s")
        );
        assert_eq!(
            parse_keyword(conf, "d").as_deref(),
            Some("https://duckduckgo.com/?q=%s")
        );
        assert_eq!(
            parse_keyword(conf, "W").as_deref(),
            parse_keyword(conf, "w").as_deref()
        );
        assert_eq!(parse_keyword(conf, "gh"), None);
        // The default-engine line is not a keyword.
        assert_eq!(parse_keyword(conf, "google"), None);
    }

    #[test]
    fn keyword_lines_do_not_break_the_default_engine() {
        let conf = "google\nw https://en.wikipedia.org/w/index.php?search=%s\n";
        assert_eq!(
            parse_template(conf).as_deref(),
            Some("https://www.google.com/search?q=%s")
        );
        // Keyword line first (misordered file): default degrades to
        // DuckDuckGo instead of a broken template.
        let misordered = "w https://en.wikipedia.org/w/index.php?search=%s\n";
        assert_eq!(parse_template(misordered), None);
    }

    #[test]
    fn quickmarks_parse_and_reject_junk() {
        let conf = "# marks\nnews https://news.ycombinator.com/\nmail https://mail.proton.me/\nbroken not-a-url\n";
        assert_eq!(
            parse_quickmark(conf, "news").as_deref(),
            Some("https://news.ycombinator.com/")
        );
        assert_eq!(
            parse_quickmark(conf, "NEWS").as_deref(),
            Some("https://news.ycombinator.com/")
        );
        assert_eq!(parse_quickmark(conf, "broken"), None);
        assert_eq!(parse_quickmark(conf, "nope"), None);
    }
}
