// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! Command palette: fuzzy search over every keymap action, so nothing
//! needs a memorized chord to be reachable. The bar hosts the UI
//! (`BarMode::Palette`); this module is the pure part — building the
//! item list from the resolved keymap and ranking items against a
//! query — so matching behavior is unit-testable without GTK.

use crate::keys::{Action, Keymap};

/// One palette entry: an action, its human description, and its
/// current chords (empty string when unbound — the palette is exactly
/// how unbound actions stay reachable).
pub struct Item {
    pub action: Action,
    pub title: &'static str,
    pub detail: String,
}

/// Every action except the palette itself, in `Action::ALL` order.
pub fn items(keymap: &Keymap) -> Vec<Item> {
    Action::ALL
        .iter()
        .filter(|a| **a != Action::CommandPalette)
        .map(|a| Item {
            action: *a,
            title: a.describe(),
            detail: keymap.chords_for(*a).join("  "),
        })
        .collect()
}

/// Rank `items` against `query`, best first. Empty query lists
/// everything in original order; non-matching items are dropped.
pub fn filter<'a>(items: &'a [Item], query: &str) -> Vec<&'a Item> {
    let query = query.trim();
    if query.is_empty() {
        return items.iter().collect();
    }
    let mut scored: Vec<(i32, usize, &Item)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            // Match against the description and the config-syntax
            // action name, so both "reload" and "hard_reload" hit.
            let hay = format!("{} {}", item.title, item.action.name());
            score(query, &hay).map(|s| (s, i, item))
        })
        .collect();
    // Stable order: score desc, then original (curated) order.
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, _, item)| item).collect()
}

/// Case-insensitive subsequence match with the usual palette biases:
/// contiguous runs and word-start hits score higher. `None` when
/// `query` is not a subsequence of `text`.
fn score(query: &str, text: &str) -> Option<i32> {
    let text: Vec<char> = text.to_lowercase().chars().collect();
    let mut total = 0i32;
    let mut pos = 0usize;
    let mut prev: Option<usize> = None;
    for qc in query.to_lowercase().chars().filter(|c| !c.is_whitespace()) {
        let found = text[pos..].iter().position(|&tc| tc == qc)? + pos;
        total += 1;
        if prev == Some(found.wrapping_sub(1)) {
            total += 2; // contiguous run
        }
        if found == 0 || !text[found - 1].is_alphanumeric() {
            total += 3; // word start
        }
        prev = Some(found);
        pos = found + 1;
    }
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_lists_every_action_but_the_palette() {
        let items = items(&Keymap::default());
        let all = filter(&items, "");
        assert_eq!(all.len(), Action::ALL.len() - 1);
        assert!(all.iter().all(|i| i.action != Action::CommandPalette));
    }

    #[test]
    fn word_start_matches_outrank_scattered_ones() {
        let items = items(&Keymap::default());
        let hits = filter(&items, "re");
        assert!(!hits.is_empty());
        // "reload page" starts a word with "re"; anything matching
        // "re" mid-word (e.g. "p_re_vious match") ranks below it.
        assert_eq!(hits[0].action, Action::Reload);
    }

    #[test]
    fn action_names_match_too() {
        let items = items(&Keymap::default());
        let hits = filter(&items, "hard_re");
        assert_eq!(hits[0].action, Action::HardReload);
    }

    #[test]
    fn non_matches_are_dropped() {
        let items = items(&Keymap::default());
        assert!(filter(&items, "qqqqzzzz").is_empty());
    }

    #[test]
    fn unbound_actions_keep_an_empty_detail() {
        let mut keymap = Keymap::default();
        keymap.apply_line("reload = none").unwrap();
        let items = items(&keymap);
        let reload = items.iter().find(|i| i.action == Action::Reload).unwrap();
        assert!(reload.detail.is_empty());
    }
}
