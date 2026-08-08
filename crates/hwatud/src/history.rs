// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! Global history + URL completion (roadmap H9).
//!
//! Every committed page-level navigation records (url, title,
//! visit_count, last_visit) in SQLite next to the cookie store. The
//! bar's URL mode and `hwatu jump`-style flows complete against it:
//! "press ctrl+l and have access to the world" is the retention
//! feature of this browser category.
//!
//! Ranking is frecency-shaped: visit count weighted by recency
//! buckets, with prefix/word-boundary bonuses at query time. All
//! scoring is pure Rust over a bounded candidate set (SQLite does the
//! coarse LIKE filter, we re-rank at most a few hundred rows).
//!
//! Ephemeral-profile daemons keep history in RAM (`:memory:`), same
//! policy as cookies and the site store.

use rusqlite::Connection;
use std::cell::RefCell;
use std::rc::Rc;

pub struct History {
    conn: RefCell<Option<Connection>>,
}

pub type Store = Rc<History>;

/// One completion candidate, ready for the bar.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub url: String,
    pub title: String,
    pub score: f64,
}

impl History {
    /// Open (or create) the history DB. `persist=false` uses an
    /// in-memory DB. A broken DB degrades to None: browsing must
    /// never fail because history is unwritable.
    pub fn load(persist: bool) -> Store {
        let conn = if persist {
            let dir = glib::user_data_dir().join("hwatud");
            let _ = std::fs::create_dir_all(&dir);
            Connection::open(dir.join("history.sqlite")).ok()
        } else {
            Connection::open_in_memory().ok()
        };
        let conn = conn.and_then(|c| {
            c.execute_batch(
                "CREATE TABLE IF NOT EXISTS visits (
                    url TEXT PRIMARY KEY,
                    title TEXT NOT NULL DEFAULT '',
                    visit_count INTEGER NOT NULL DEFAULT 0,
                    last_visit INTEGER NOT NULL DEFAULT 0
                );
                CREATE INDEX IF NOT EXISTS visits_last ON visits(last_visit DESC);",
            )
            .ok()
            .map(|_| c)
        });
        Rc::new(History {
            conn: RefCell::new(conn),
        })
    }

    #[cfg(test)]
    pub fn in_memory() -> Store {
        Self::load(false)
    }

    /// Record one committed navigation. Internal pages and blanks are
    /// the caller's problem to filter (they know what a launcher is).
    pub fn record_visit(&self, url: &str) {
        let Some(conn) = &*self.conn.borrow() else {
            return;
        };
        let now = now_secs();
        let _ = conn.execute(
            "INSERT INTO visits (url, visit_count, last_visit) VALUES (?1, 1, ?2)
             ON CONFLICT(url) DO UPDATE SET
                visit_count = visit_count + 1,
                last_visit = ?2",
            rusqlite::params![url, now],
        );
    }

    /// Attach/refresh the title once the page delivers it.
    pub fn record_title(&self, url: &str, title: &str) {
        if title.is_empty() {
            return;
        }
        let Some(conn) = &*self.conn.borrow() else {
            return;
        };
        let _ = conn.execute(
            "UPDATE visits SET title = ?2 WHERE url = ?1",
            rusqlite::params![url, title],
        );
    }

    /// Complete `query` against history, best first, at most `limit`.
    /// Empty query returns the frecency top of the world.
    pub fn complete(&self, query: &str, limit: usize) -> Vec<Hit> {
        let Some(conn) = &*self.conn.borrow() else {
            return Vec::new();
        };
        // Coarse filter in SQL (bounded), fine ranking in Rust.
        let candidates: Vec<(String, String, i64, i64)> = if query.is_empty() {
            query_rows(
                conn,
                "SELECT url, title, visit_count, last_visit FROM visits
                 ORDER BY last_visit DESC LIMIT 400",
                [],
            )
        } else {
            let like = format!("%{}%", query.replace('%', "\\%").replace('_', "\\_"));
            query_rows(
                conn,
                "SELECT url, title, visit_count, last_visit FROM visits
                 WHERE url LIKE ?1 ESCAPE '\\' OR title LIKE ?1 ESCAPE '\\'
                 ORDER BY last_visit DESC LIMIT 400",
                rusqlite::params![like],
            )
        };
        let now = now_secs();
        let mut hits: Vec<Hit> = candidates
            .into_iter()
            .filter_map(|(url, title, count, last)| {
                let score = score(query, &url, &title, count, last, now)?;
                Some(Hit { url, title, score })
            })
            .collect();
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(limit);
        hits
    }

    /// Delete all history (the privacy verb).
    pub fn clear(&self) -> usize {
        let Some(conn) = &*self.conn.borrow() else {
            return 0;
        };
        conn.execute("DELETE FROM visits", []).unwrap_or(0)
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Frecency-with-match-quality. None = no textual match for a
/// non-empty query (the SQL LIKE is case-insensitive ASCII only; this
/// re-check keeps unicode queries honest).
fn score(
    query: &str,
    url: &str,
    title: &str,
    count: i64,
    last_visit: i64,
    now: i64,
) -> Option<f64> {
    let match_bonus = if query.is_empty() {
        1.0
    } else {
        let q = query.to_lowercase();
        let url_l = url.to_lowercase();
        let title_l = title.to_lowercase();
        // Strongest: the host or a path/word boundary starts with the
        // query (typing "gith" should surface github.com over a page
        // that merely mentions it).
        let stripped = url_l
            .strip_prefix("https://")
            .or_else(|| url_l.strip_prefix("http://"))
            .unwrap_or(&url_l);
        let stripped = stripped.strip_prefix("www.").unwrap_or(stripped);
        if stripped.starts_with(&q) {
            3.0
        } else if stripped
            .split(['/', '.', '-', '_', '?', '=', '&'])
            .any(|w| w.starts_with(&q))
            || title_l
                .split([' ', '-', ':', '·', '|'])
                .any(|w| w.starts_with(&q))
        {
            2.0
        } else if url_l.contains(&q) || title_l.contains(&q) {
            1.0
        } else {
            return None;
        }
    };
    // Firefox-style frecency buckets: recent visits count for more.
    let age_days = ((now - last_visit).max(0)) as f64 / 86_400.0;
    let recency = if age_days < 4.0 {
        1.0
    } else if age_days < 14.0 {
        0.7
    } else if age_days < 31.0 {
        0.5
    } else if age_days < 90.0 {
        0.3
    } else {
        0.1
    };
    Some(match_bonus * (count as f64).ln_1p() * recency + match_bonus * 0.01)
}

fn query_rows(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Vec<(String, String, i64, i64)> {
    let Ok(mut stmt) = conn.prepare(sql) else {
        return Vec::new();
    };
    let rows = stmt.query_map(params, |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
        ))
    });
    match rows {
        Ok(rows) => rows.flatten().collect(),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_completes() {
        let h = History::in_memory();
        h.record_visit("https://github.com/rust-lang/rust");
        h.record_visit("https://github.com/rust-lang/rust");
        h.record_title("https://github.com/rust-lang/rust", "rust-lang/rust: Rust");
        h.record_visit("https://example.com/");

        let hits = h.complete("gith", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].url, "https://github.com/rust-lang/rust");
        assert_eq!(hits[0].title, "rust-lang/rust: Rust");

        // Empty query returns everything, most-relevant first.
        let all = h.complete("", 10);
        assert_eq!(all.len(), 2);
        // github has 2 visits, example 1: frecency puts github first.
        assert_eq!(all[0].url, "https://github.com/rust-lang/rust");
    }

    #[test]
    fn host_prefix_outranks_mere_mention() {
        let h = History::in_memory();
        // Page that merely mentions "git" in the path, visited often.
        for _ in 0..5 {
            h.record_visit("https://news.site/story-about-git");
        }
        // The actual host match, visited once.
        h.record_visit("https://github.com/");
        let hits = h.complete("github", 10);
        assert_eq!(hits[0].url, "https://github.com/");
    }

    #[test]
    fn www_and_scheme_stripped_for_prefix_match() {
        let h = History::in_memory();
        h.record_visit("https://www.wikipedia.org/");
        h.record_visit("https://blog.site/why-wikipedia-matters");
        let hits = h.complete("wiki", 10);
        assert_eq!(hits.len(), 2);
        // Host-prefix match (scheme + www stripped) outranks the page
        // that merely contains the query in its path.
        assert_eq!(hits[0].url, "https://www.wikipedia.org/");
        assert!(hits[0].score > hits[1].score);
    }

    #[test]
    fn title_word_matches() {
        let h = History::in_memory();
        h.record_visit("https://doc.rust-lang.org/book/");
        h.record_title(
            "https://doc.rust-lang.org/book/",
            "The Rust Programming Language",
        );
        let hits = h.complete("programming", 10);
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn no_match_returns_empty_and_clear_wipes() {
        let h = History::in_memory();
        h.record_visit("https://example.com/");
        assert!(h.complete("zzzznope", 10).is_empty());
        assert_eq!(h.clear(), 1);
        assert!(h.complete("", 10).is_empty());
    }

    #[test]
    fn like_wildcards_are_escaped() {
        let h = History::in_memory();
        h.record_visit("https://example.com/a");
        // A bare "%" must not match everything.
        assert!(h.complete("%", 10).is_empty());
        assert!(h.complete("_", 10).is_empty());
    }
}
