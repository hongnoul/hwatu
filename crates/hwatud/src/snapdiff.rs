// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! `hwatu snapshot --diff`: what changed since the last diff snapshot.
//!
//! A full snapshot of a busy page costs thousands of tokens; in an
//! agent's iterate loop ("edit code, reload, look again") almost all
//! of it is repetition. This module normalizes a snapshot into an
//! ordered list of [`Node`]s (url, title, visible text lines, one
//! node per interactable) and diffs two such lists into
//! `{added, removed, changed, unchanged_count}`.
//!
//! The diff is an LCS over `(key, content)` pairs, then a second pass
//! that pairs leftover nodes by identity key:
//!
//! * in the LCS: **unchanged** (only counted, never listed);
//! * same key on both sides, different content: **changed**
//!   (`{key, old, new}`);
//! * same key *and* content but outside the LCS (it jumped across
//!   other nodes): **changed** with `"moved": true`;
//! * everything else: **added** / **removed**.
//!
//! Identity keys prefer stable DOM anchors (`id`, then `name`, then
//! `href`), falling back to `tag:text`. A node identified only by its
//! text therefore reports a text edit as removed+added, exactly like
//! a line-based diff; nodes with a stable anchor report it as changed.

use serde_json::{json, Value};

/// One normalized snapshot node. Equality (identity + content) drives
/// the LCS; `emit` additionally carries the live `ref` index of the
/// new snapshot, which is excluded from comparison because a pure
/// insertion renumbers every later ref without changing anything the
/// agent cares about.
#[derive(Clone, Debug)]
pub struct Node {
    /// Identity: what makes this "the same node" across snapshots.
    key: String,
    /// Ref-less content compared for change detection.
    bare: Value,
    /// Content as reported to the agent (interactables keep `ref`).
    emit: Value,
}

impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.bare == other.bare
    }
}

/// Flatten a full snapshot value (the JSON produced by the snapshot
/// JS) into an ordered node list.
pub fn normalize(snapshot: &Value) -> Vec<Node> {
    let mut out = Vec::new();
    if let Some(url) = snapshot.get("url").and_then(Value::as_str) {
        let v = json!({ "kind": "url", "url": url });
        out.push(Node {
            key: "url".into(),
            bare: v.clone(),
            emit: v,
        });
    }
    if let Some(title) = snapshot.get("title").and_then(Value::as_str) {
        let v = json!({ "kind": "title", "title": title });
        out.push(Node {
            key: "title".into(),
            bare: v.clone(),
            emit: v,
        });
    }
    let mut seen = std::collections::HashMap::new();
    if let Some(text) = snapshot.get("text").and_then(Value::as_str) {
        for line in text.split('\n').map(str::trim).filter(|l| !l.is_empty()) {
            let key = occurrence(&mut seen, format!("text:{line}"));
            let v = json!({ "kind": "text", "text": line });
            out.push(Node {
                key,
                bare: v.clone(),
                emit: v,
            });
        }
    }
    if let Some(els) = snapshot.get("interactables").and_then(Value::as_array) {
        for el in els {
            let key = occurrence(&mut seen, interactable_key(el));
            let mut bare = el.clone();
            if let Some(map) = bare.as_object_mut() {
                map.remove("ref");
            }
            out.push(Node {
                key,
                bare,
                emit: el.clone(),
            });
        }
    }
    out
}

/// Stable-ish identity for one interactable: prefer DOM anchors that
/// survive text edits, fall back to the visible label.
fn interactable_key(el: &Value) -> String {
    let tag = el.get("tag").and_then(Value::as_str).unwrap_or("?");
    if let Some(id) = el.get("id").and_then(Value::as_str) {
        return format!("{tag}#{id}");
    }
    if let Some(name) = el.get("name").and_then(Value::as_str) {
        return format!("{tag}[name={name}]");
    }
    if let Some(href) = el.get("href").and_then(Value::as_str) {
        return format!("{tag}[href={href}]");
    }
    let text = el.get("text").and_then(Value::as_str).unwrap_or("");
    format!("{tag}:{text}")
}

/// Disambiguate repeated keys ("text:Buy" three times) by occurrence
/// index so the n-th copy pairs with the n-th copy.
fn occurrence(seen: &mut std::collections::HashMap<String, u32>, key: String) -> String {
    let n = seen.entry(key.clone()).or_insert(0);
    *n += 1;
    if *n == 1 {
        key
    } else {
        format!("{key}~{n}")
    }
}

/// Diff two normalized node lists into the wire shape:
/// `{added: [...], removed: [...], changed: [...], unchanged_count: N}`.
pub fn diff(old: &[Node], new: &[Node]) -> Value {
    // LCS over full node equality (key + content): the ordered common
    // core is "unchanged".
    let n = old.len();
    let m = new.len();
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if old[i] == new[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut unmatched_old = Vec::new();
    let mut unmatched_new = Vec::new();
    let mut unchanged = 0usize;
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if old[i] == new[j] {
            unchanged += 1;
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            unmatched_old.push(i);
            i += 1;
        } else {
            unmatched_new.push(j);
            j += 1;
        }
    }
    unmatched_old.extend(i..n);
    unmatched_new.extend(j..m);

    // Second pass: pair leftovers by identity key. Same key, new
    // content = changed; same key and content (but outside the LCS)
    // = moved; unpaired = added/removed.
    let mut changed = Vec::new();
    let mut removed = Vec::new();
    let mut added = Vec::new();
    let mut used = vec![false; unmatched_new.len()];
    for &oi in &unmatched_old {
        let pair = unmatched_new
            .iter()
            .enumerate()
            .find(|(slot, &nj)| !used[*slot] && new[nj].key == old[oi].key);
        match pair {
            Some((slot, &nj)) => {
                used[slot] = true;
                if old[oi].bare == new[nj].bare {
                    changed.push(json!({
                        "key": new[nj].key,
                        "moved": true,
                        "node": new[nj].emit,
                    }));
                } else {
                    changed.push(json!({
                        "key": new[nj].key,
                        "old": old[oi].bare,
                        "new": new[nj].emit,
                    }));
                }
            }
            None => removed.push(old[oi].bare.clone()),
        }
    }
    for (slot, &nj) in unmatched_new.iter().enumerate() {
        if !used[slot] {
            added.push(new[nj].emit.clone());
        }
    }
    json!({
        "added": added,
        "removed": removed,
        "changed": changed,
        "unchanged_count": unchanged,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(text: &str, els: Value) -> Value {
        json!({
            "url": "https://example.test/",
            "title": "fixture",
            "text": text,
            "interactables": els,
        })
    }

    fn d(old: &Value, new: &Value) -> Value {
        diff(&normalize(old), &normalize(new))
    }

    fn len(v: &Value, k: &str) -> usize {
        v[k].as_array().map(Vec::len).unwrap_or(0)
    }

    #[test]
    fn no_change_is_empty() {
        let a = snap(
            "hello\nworld",
            json!([{ "ref": 0, "tag": "button", "id": "go", "text": "Go" }]),
        );
        let out = d(&a, &a);
        assert_eq!(len(&out, "added"), 0, "{out}");
        assert_eq!(len(&out, "removed"), 0);
        assert_eq!(len(&out, "changed"), 0);
        // url + title + 2 text lines + 1 interactable
        assert_eq!(out["unchanged_count"], 5);
    }

    #[test]
    fn insertion_is_added_only() {
        let a = snap(
            "hello",
            json!([{ "ref": 0, "tag": "button", "id": "go", "text": "Go" }]),
        );
        let b = snap(
            "hello",
            json!([
                { "ref": 0, "tag": "a", "id": "new", "text": "New", "href": "https://x/" },
                { "ref": 1, "tag": "button", "id": "go", "text": "Go" },
            ]),
        );
        let out = d(&a, &b);
        assert_eq!(len(&out, "added"), 1, "{out}");
        assert_eq!(out["added"][0]["id"], "new");
        // The insertion renumbered the button's ref (0 -> 1); that
        // must not surface as a change.
        assert_eq!(len(&out, "removed"), 0);
        assert_eq!(len(&out, "changed"), 0);
        assert_eq!(out["unchanged_count"], 4);
    }

    #[test]
    fn removal_is_removed_only() {
        let a = snap(
            "hello",
            json!([
                { "ref": 0, "tag": "button", "id": "go", "text": "Go" },
                { "ref": 1, "tag": "button", "id": "stop", "text": "Stop" },
            ]),
        );
        let b = snap(
            "hello",
            json!([{ "ref": 0, "tag": "button", "id": "go", "text": "Go" }]),
        );
        let out = d(&a, &b);
        assert_eq!(len(&out, "removed"), 1, "{out}");
        assert_eq!(out["removed"][0]["id"], "stop");
        assert!(
            out["removed"][0].get("ref").is_none(),
            "removed nodes must not advertise a stale ref"
        );
        assert_eq!(len(&out, "added"), 0);
        assert_eq!(len(&out, "changed"), 0);
    }

    #[test]
    fn text_change_on_anchored_node_is_changed() {
        let a = snap(
            "x",
            json!([{ "ref": 0, "tag": "button", "id": "go", "text": "Go" }]),
        );
        let b = snap(
            "x",
            json!([{ "ref": 0, "tag": "button", "id": "go", "text": "Really go" }]),
        );
        let out = d(&a, &b);
        assert_eq!(len(&out, "changed"), 1, "{out}");
        assert_eq!(out["changed"][0]["key"], "button#go");
        assert_eq!(out["changed"][0]["old"]["text"], "Go");
        assert_eq!(out["changed"][0]["new"]["text"], "Really go");
        assert_eq!(
            out["changed"][0]["new"]["ref"], 0,
            "new side keeps the live ref"
        );
        assert_eq!(len(&out, "added"), 0);
        assert_eq!(len(&out, "removed"), 0);
    }

    #[test]
    fn text_line_edit_is_removed_plus_added() {
        let a = snap("stable\nold line", json!([]));
        let b = snap("stable\nnew line", json!([]));
        let out = d(&a, &b);
        assert_eq!(len(&out, "removed"), 1, "{out}");
        assert_eq!(out["removed"][0]["text"], "old line");
        assert_eq!(len(&out, "added"), 1);
        assert_eq!(out["added"][0]["text"], "new line");
        assert_eq!(len(&out, "changed"), 0);
        assert_eq!(out["unchanged_count"], 3); // url + title + "stable"
    }

    #[test]
    fn attribute_change_is_changed() {
        let a = snap(
            "x",
            json!([{ "ref": 0, "tag": "input", "id": "opt", "type": "checkbox" }]),
        );
        let b = snap(
            "x",
            json!([{ "ref": 0, "tag": "input", "id": "opt", "type": "checkbox", "checked": true }]),
        );
        let out = d(&a, &b);
        assert_eq!(len(&out, "changed"), 1, "{out}");
        assert_eq!(out["changed"][0]["new"]["checked"], true);
        assert_eq!(len(&out, "added"), 0);
        assert_eq!(len(&out, "removed"), 0);
    }

    #[test]
    fn reorder_is_moved_not_add_remove() {
        let a = snap(
            "x",
            json!([
                { "ref": 0, "tag": "button", "id": "one", "text": "One" },
                { "ref": 1, "tag": "button", "id": "two", "text": "Two" },
                { "ref": 2, "tag": "button", "id": "three", "text": "Three" },
            ]),
        );
        let b = snap(
            "x",
            json!([
                { "ref": 0, "tag": "button", "id": "three", "text": "Three" },
                { "ref": 1, "tag": "button", "id": "one", "text": "One" },
                { "ref": 2, "tag": "button", "id": "two", "text": "Two" },
            ]),
        );
        let out = d(&a, &b);
        assert_eq!(len(&out, "added"), 0, "{out}");
        assert_eq!(len(&out, "removed"), 0);
        assert_eq!(len(&out, "changed"), 1);
        assert_eq!(out["changed"][0]["key"], "button#three");
        assert_eq!(out["changed"][0]["moved"], true);
    }

    #[test]
    fn duplicate_labels_pair_by_occurrence() {
        let a = snap(
            "x",
            json!([
                { "ref": 0, "tag": "button", "text": "Buy" },
                { "ref": 1, "tag": "button", "text": "Buy" },
            ]),
        );
        let out = d(&a, &a);
        assert_eq!(len(&out, "changed"), 0, "{out}");
        assert_eq!(out["unchanged_count"], 5);
    }

    #[test]
    fn url_change_without_navigation_is_changed() {
        let a = snap("x", json!([]));
        let mut b = a.clone();
        b["url"] = json!("https://example.test/#tab2");
        let out = d(&a, &b);
        assert_eq!(len(&out, "changed"), 1, "{out}");
        assert_eq!(out["changed"][0]["key"], "url");
    }
}
