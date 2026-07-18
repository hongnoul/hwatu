//! Adblock Plus filter syntax -> WebKit content-blocker JSON.
//!
//! WebKit (like Safari) evaluates content-blocker rules natively in the
//! network process: no JavaScript in the hot path, no extension
//! machinery. This module converts the common subset of ABP/EasyList
//! syntax into that JSON format. Rules that cannot be expressed
//! declaratively ($csp, $redirect, scriptlets, procedural cosmetics,
//! raw regex filters) are skipped rather than approximated, so a bad
//! filter can never break page loads or fail ruleset compilation.
//!
//! Output ordering follows the content-blocker contract:
//! css-display-none rules, then block rules, then
//! ignore-previous-rules (exceptions) last.

use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashSet};

/// Max selectors merged into one css-display-none rule. Grouping
//  generic selectors keeps the compiled rule count (and DFA) small.
const SELECTOR_CHUNK: usize = 250;

/// Domain scope of a cosmetic rule: (if-domains, unless-domains).
type Scope = (Vec<String>, Vec<String>);

pub struct Converted {
    pub json: String,
    /// Number of compiled content-blocker rules.
    pub rules: usize,
    /// Source lines skipped as unsupported.
    pub skipped: usize,
}

pub fn convert<'a, I>(lines: I) -> Converted
where
    I: IntoIterator<Item = &'a str>,
{
    let mut skipped = 0usize;

    // Cosmetic rules grouped by domain scope so selectors can be merged.
    // Scope domains are sorted for stable output.
    let mut cosmetic: BTreeMap<Scope, Vec<String>> = BTreeMap::new();
    // Selectors excepted somewhere (#@#). Dropped globally: slightly
    // under-hides, never breaks a site that legitimately unhides.
    let mut cosmetic_exceptions: HashSet<String> = HashSet::new();

    let mut blocks: Vec<Value> = Vec::new();
    let mut exceptions: Vec<Value> = Vec::new();

    for raw in lines {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('!') || line.starts_with('[') {
            continue;
        }
        if line.len() > 4096 {
            skipped += 1;
            continue;
        }

        // Cosmetic exception: remember the selector, drop it everywhere.
        if let Some(pos) = line.find("#@#") {
            cosmetic_exceptions.insert(line[pos + 3..].trim().to_string());
            continue;
        }
        // Procedural / snippet cosmetic variants: not expressible.
        if line.contains("#?#") || line.contains("#$#") || line.contains("##^") {
            skipped += 1;
            continue;
        }

        if let Some(pos) = line.find("##") {
            let (domains, selector) = (&line[..pos], line[pos + 2..].trim());
            match parse_cosmetic(domains, selector) {
                Some((scope, sel)) => cosmetic.entry(scope).or_default().push(sel),
                None => skipped += 1,
            }
            continue;
        }

        match parse_network(line) {
            NetRule::Block(v) => blocks.push(v),
            NetRule::Exception(v) => exceptions.push(v),
            NetRule::Skip => skipped += 1,
        }
    }

    let mut rules: Vec<Value> = Vec::new();
    for ((if_dom, unless_dom), selectors) in &cosmetic {
        let mut selectors: Vec<&String> = selectors
            .iter()
            .filter(|s| !cosmetic_exceptions.contains(s.as_str()))
            .collect();
        selectors.sort();
        selectors.dedup();
        for chunk in selectors.chunks(SELECTOR_CHUNK) {
            let mut trigger = Map::new();
            trigger.insert("url-filter".into(), json!(".*"));
            if !if_dom.is_empty() {
                trigger.insert("if-domain".into(), json!(if_dom));
            } else if !unless_dom.is_empty() {
                trigger.insert("unless-domain".into(), json!(unless_dom));
            }
            let selector = chunk
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            rules.push(json!({
                "trigger": Value::Object(trigger),
                "action": {"type": "css-display-none", "selector": selector},
            }));
        }
    }
    rules.append(&mut blocks);
    rules.append(&mut exceptions);

    Converted {
        rules: rules.len(),
        json: serde_json::to_string(&rules).unwrap_or_else(|_| "[]".into()),
        skipped,
    }
}

/// Parse `domains##selector`. Returns the domain scope and selector,
/// or None when the selector is procedural/unsafe.
fn parse_cosmetic(domains: &str, selector: &str) -> Option<(Scope, String)> {
    if selector.is_empty()
        || selector.starts_with("+js(")
        || selector.contains(":-abp-")
        || selector.contains("[-ext-")
        || selector.contains(":has-text(")
        || selector.contains(":matches-css")
        || selector.contains(":xpath(")
        || selector.contains(":style(")
        || selector.contains(":remove(")
        || selector.contains('\\')
    {
        return None;
    }
    let (mut incl, mut excl) = (Vec::new(), Vec::new());
    for d in domains.split(',').map(str::trim).filter(|d| !d.is_empty()) {
        match d.strip_prefix('~') {
            Some(neg) => excl.push(format!("*{}", neg.to_ascii_lowercase())),
            None => incl.push(format!("*{}", d.to_ascii_lowercase())),
        }
    }
    // WebKit triggers cannot carry both; positives dominate.
    if !incl.is_empty() {
        excl.clear();
    }
    incl.sort();
    excl.sort();
    Some(((incl, excl), selector.to_string()))
}

enum NetRule {
    Block(Value),
    Exception(Value),
    Skip,
}

#[derive(Default)]
struct NetOpts {
    types: Vec<&'static str>,
    neg_types: Vec<&'static str>,
    load_type: Option<&'static str>,
    if_domain: Vec<String>,
    unless_domain: Vec<String>,
    case_sensitive: bool,
    /// $document: whole-page semantics (used by exceptions).
    document: bool,
    /// A requested type was widened to "raw" (ping -> raw etc.). Safe
    /// only while a URL pattern narrows the match; see parse_network.
    broadened: bool,
}

/// All resource types WebKitGTK's content extensions accept. Kept
/// conservative: unknown strings fail ruleset compilation outright.
const ALL_TYPES: &[&str] = &[
    "document",
    "image",
    "style-sheet",
    "script",
    "font",
    "raw",
    "svg-document",
    "media",
    "popup",
];

fn parse_network(line: &str) -> NetRule {
    let (exception, line) = match line.strip_prefix("@@") {
        Some(rest) => (true, rest),
        None => (false, line),
    };

    // Raw regex filters: WebKit's url-filter regex subset differs from
    // full PCRE and one invalid pattern fails the whole compilation.
    if line.len() > 1 && line.starts_with('/') && line.ends_with('/') {
        return NetRule::Skip;
    }

    let (pattern, opts) = split_options(line);
    let opts = match opts.map(parse_options) {
        Some(Some(o)) => o,
        Some(None) => return NetRule::Skip, // unsupported option
        None => NetOpts::default(),
    };

    // `$document` exception == whitelist the whole site. Express it as
    // an if-domain ignore rule so it cancels every earlier rule there.
    if exception && opts.document {
        if let Some(domain) = plain_domain(pattern) {
            return NetRule::Exception(json!({
                "trigger": {"url-filter": ".*", "if-domain": [format!("*{domain}")]},
                "action": {"type": "ignore-previous-rules"},
            }));
        }
    }

    let has_scope = !opts.if_domain.is_empty()
        || !opts.unless_domain.is_empty()
        || !opts.types.is_empty()
        || !opts.neg_types.is_empty()
        || opts.load_type.is_some();
    let bare = pattern.trim_matches('*');
    if bare.is_empty() && !has_scope {
        return NetRule::Skip; // would match everything
    }
    // A pattern-less rule relies entirely on its type scope. If that
    // scope was broadened (e.g. EasyList's `$ping,third-party` widened
    // to "raw", which also covers XHR/fetch), blocking it would break
    // legitimate requests across the web. Skip; pattern-less rules
    // must carry their exact meaning or nothing.
    if bare.is_empty() && opts.broadened {
        return NetRule::Skip;
    }

    let Some(regex) = pattern_to_regex(pattern) else {
        return NetRule::Skip;
    };

    let mut trigger = Map::new();
    trigger.insert("url-filter".into(), json!(regex));
    if opts.case_sensitive {
        trigger.insert("url-filter-is-case-sensitive".into(), json!(true));
    }
    let types = resolve_types(&opts);
    if let Some(types) = types {
        if types.is_empty() {
            return NetRule::Skip; // negations cancelled everything
        }
        trigger.insert("resource-type".into(), json!(types));
    }
    if let Some(lt) = opts.load_type {
        trigger.insert("load-type".into(), json!([lt]));
    }
    if !opts.if_domain.is_empty() {
        trigger.insert("if-domain".into(), json!(opts.if_domain));
    } else if !opts.unless_domain.is_empty() {
        trigger.insert("unless-domain".into(), json!(opts.unless_domain));
    }

    let action = if exception {
        "ignore-previous-rules"
    } else {
        "block"
    };
    let rule = json!({
        "trigger": Value::Object(trigger),
        "action": {"type": action},
    });
    if exception {
        NetRule::Exception(rule)
    } else {
        NetRule::Block(rule)
    }
}

/// None = match all types (omit the key). Some(list) = restricted.
fn resolve_types(opts: &NetOpts) -> Option<Vec<&'static str>> {
    if opts.types.is_empty() && opts.neg_types.is_empty() {
        return None;
    }
    let base: Vec<&'static str> = if opts.types.is_empty() {
        ALL_TYPES.to_vec()
    } else {
        opts.types.clone()
    };
    let mut out: Vec<&'static str> = base
        .into_iter()
        .filter(|t| !opts.neg_types.contains(t))
        .collect();
    out.sort();
    out.dedup();
    Some(out)
}

/// Split `pattern$options`, being careful not to treat `$` inside the
/// pattern as an option separator.
fn split_options(line: &str) -> (&str, Option<&str>) {
    if let Some(pos) = line.rfind('$') {
        let opts = &line[pos + 1..];
        let looks_like_opts = !opts.is_empty()
            && opts.split(',').all(|tok| {
                let tok = tok.trim().trim_start_matches('~');
                let name = tok.split('=').next().unwrap_or("");
                !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            });
        if looks_like_opts {
            return (&line[..pos], Some(opts));
        }
    }
    (line, None)
}

/// Returns None when an option makes the rule inexpressible.
fn parse_options(opts: &str) -> Option<NetOpts> {
    let mut out = NetOpts::default();
    for tok in opts.split(',').map(str::trim) {
        let (neg, tok) = match tok.strip_prefix('~') {
            Some(rest) => (true, rest),
            None => (false, tok),
        };
        let (name, value) = match tok.split_once('=') {
            Some((n, v)) => (n, Some(v)),
            None => (tok, None),
        };
        let mapped = match name {
            "script" => Some("script"),
            "image" | "background" => Some("image"),
            "stylesheet" | "css" => Some("style-sheet"),
            "subdocument" | "frame" => Some("document"),
            "font" => Some("font"),
            "media" => Some("media"),
            "popup" => Some("popup"),
            // "raw" covers XHR/fetch and friends. xhr maps ~exactly;
            // the rest are wider than the author asked for.
            "xmlhttprequest" | "xhr" => Some("raw"),
            "websocket" | "ping" | "beacon" | "other" | "object" | "object-subrequest" => {
                out.broadened = true;
                Some("raw")
            }
            _ => None,
        };
        if let Some(t) = mapped {
            if neg {
                out.neg_types.push(t);
            } else {
                out.types.push(t);
            }
            continue;
        }
        match name {
            "document" | "doc" => {
                if neg {
                    out.neg_types.push("document");
                } else {
                    out.types.push("document");
                    out.document = true;
                }
            }
            "third-party" | "3p" => {
                out.load_type = Some(if neg { "first-party" } else { "third-party" })
            }
            "first-party" | "1p" => {
                out.load_type = Some(if neg { "third-party" } else { "first-party" })
            }
            "match-case" => out.case_sensitive = true,
            "domain" | "from" => {
                for d in value?.split('|').map(str::trim).filter(|d| !d.is_empty()) {
                    match d.strip_prefix('~') {
                        Some(nd) => out
                            .unless_domain
                            .push(format!("*{}", nd.to_ascii_lowercase())),
                        None => out.if_domain.push(format!("*{}", d.to_ascii_lowercase())),
                    }
                }
            }
            // Harmless priority/metadata hints: blocking already wins.
            "important" | "all" => {}
            // Everything else ($csp, $redirect, $removeparam, $replace,
            // $generichide, $method, $header, $denyallow, ...) cannot be
            // expressed; drop the rule rather than approximate it.
            _ => return None,
        }
    }
    // A trigger cannot carry both domain keys; positives dominate.
    if !out.if_domain.is_empty() {
        out.unless_domain.clear();
    }
    out.if_domain.sort();
    out.if_domain.dedup();
    out.unless_domain.sort();
    out.unless_domain.dedup();
    Some(out)
}

/// `||example.com^` (or with trailing `/` or `|`) -> `example.com`.
fn plain_domain(pattern: &str) -> Option<String> {
    let host = pattern
        .strip_prefix("||")?
        .trim_end_matches('|')
        .trim_end_matches('^')
        .trim_end_matches('/');
    let ok = !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    ok.then(|| host.to_ascii_lowercase())
}

/// ABP separator `^`: proven-safe class used by the AdGuard converter.
const SEPARATOR: &str = "[/:&?]?";
/// `||` anchor: scheme, then any chain of subdomains.
const DOMAIN_ANCHOR: &str = "^[htpsw]+:\\/\\/([a-z0-9_-]+\\.)*";

/// Convert an ABP address pattern to WebKit's url-filter regex subset.
fn pattern_to_regex(pattern: &str) -> Option<String> {
    let (domain_anchored, rest) = match pattern.strip_prefix("||") {
        Some(r) => (true, r),
        None => (false, pattern),
    };
    let (start_anchored, rest) = if domain_anchored {
        (false, rest)
    } else {
        match rest.strip_prefix('|') {
            Some(r) => (true, r),
            None => (false, rest),
        }
    };
    let (end_anchored, rest) = match rest.strip_suffix('|') {
        Some(r) => (true, r),
        None => (false, rest),
    };
    if rest.contains('|') {
        return None; // interior pipe: malformed/unsupported
    }

    let mut re = String::with_capacity(rest.len() + 32);
    if domain_anchored {
        re.push_str(DOMAIN_ANCHOR);
    } else if start_anchored {
        re.push('^');
    }
    for c in rest.chars() {
        match c {
            '*' => re.push_str(".*"),
            '^' => re.push_str(SEPARATOR),
            '.' | '+' | '?' | '$' | '{' | '}' | '(' | ')' | '[' | ']' | '\\' | '/' => {
                re.push('\\');
                re.push(c);
            }
            c if c.is_ascii() => re.push(c),
            // Non-ASCII in url-filter fails compilation; hosts in
            // filter lists are punycode already, so just skip.
            _ => return None,
        }
    }
    if end_anchored {
        re.push('$');
    }
    // Option-only rules (`$popup,domain=...`) leave the pattern empty;
    // WebKit rejects an empty url-filter, so match-anything explicitly.
    if re.is_empty() {
        re.push_str(".*");
    }
    Some(re)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn rules(list: &str) -> Vec<Value> {
        let c = convert(list.lines());
        serde_json::from_str(&c.json).expect("valid JSON")
    }

    fn one(list: &str) -> Value {
        let mut r = rules(list);
        assert_eq!(r.len(), 1, "expected exactly one rule for {list:?}");
        r.remove(0)
    }

    #[test]
    fn domain_anchor_and_separator() {
        let r = one("||ads.example.com^");
        assert_eq!(
            r["trigger"]["url-filter"],
            "^[htpsw]+:\\/\\/([a-z0-9_-]+\\.)*ads\\.example\\.com[/:&?]?"
        );
        assert_eq!(r["action"]["type"], "block");
    }

    #[test]
    fn start_and_end_anchors() {
        let r = one("|https://tracker.io/px|");
        assert_eq!(
            r["trigger"]["url-filter"],
            "^https:\\/\\/tracker\\.io\\/px$"
        );
    }

    #[test]
    fn wildcard_and_escaping() {
        let r = one("/banner/*/ad.");
        assert_eq!(r["trigger"]["url-filter"], "\\/banner\\/.*\\/ad\\.");
    }

    #[test]
    fn type_and_party_options() {
        let r = one("||adnet.com^$script,image,third-party");
        let types = r["trigger"]["resource-type"].as_array().unwrap();
        assert!(types.contains(&Value::from("script")));
        assert!(types.contains(&Value::from("image")));
        assert_eq!(types.len(), 2);
        assert_eq!(
            r["trigger"]["load-type"],
            serde_json::json!(["third-party"])
        );
    }

    #[test]
    fn negated_types_complement() {
        let r = one("||adnet.com^$~script");
        let types = r["trigger"]["resource-type"].as_array().unwrap();
        assert!(!types.contains(&Value::from("script")));
        assert!(types.contains(&Value::from("image")));
    }

    #[test]
    fn domain_option_scoping() {
        let r = one("||cdn.com/ad.js$domain=news.com|~blog.news.com");
        // Positives dominate; negatives dropped when mixed.
        assert_eq!(r["trigger"]["if-domain"], serde_json::json!(["*news.com"]));
        assert!(r["trigger"].get("unless-domain").is_none());
    }

    #[test]
    fn exception_orders_last() {
        let list = "@@||good.com^\n||bad.com^";
        let r = rules(list);
        assert_eq!(r[0]["action"]["type"], "block");
        assert_eq!(r[1]["action"]["type"], "ignore-previous-rules");
    }

    #[test]
    fn document_exception_becomes_if_domain() {
        let r = one("@@||trusted.org^$document");
        assert_eq!(r["action"]["type"], "ignore-previous-rules");
        assert_eq!(r["trigger"]["url-filter"], ".*");
        assert_eq!(
            r["trigger"]["if-domain"],
            serde_json::json!(["*trusted.org"])
        );
    }

    #[test]
    fn cosmetic_generic_and_scoped() {
        let list = "##.ad-banner\nexample.com###sidebar-ads";
        let r = rules(list);
        assert_eq!(r.len(), 2);
        // Generic (no domain scope) and scoped rule both css-display-none.
        assert!(r.iter().all(|v| v["action"]["type"] == "css-display-none"));
        let scoped = r
            .iter()
            .find(|v| v["trigger"].get("if-domain").is_some())
            .unwrap();
        assert_eq!(
            scoped["trigger"]["if-domain"],
            serde_json::json!(["*example.com"])
        );
        assert_eq!(scoped["action"]["selector"], "#sidebar-ads");
    }

    #[test]
    fn cosmetic_exception_drops_selector() {
        let list = "##.promo\nsite.com#@#.promo";
        let c = convert(list.lines());
        assert_eq!(c.rules, 0);
    }

    #[test]
    fn cosmetic_selectors_merge() {
        let list = "##.a1\n##.a2\n##.a3";
        let r = rules(list);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0]["action"]["selector"], ".a1, .a2, .a3");
    }

    #[test]
    fn unsupported_rules_skipped_safely() {
        let list = "\
||x.com^$csp=script-src 'none'
||y.com^$redirect=noopjs
/^https?:\\/\\/regex/
example.com##+js(nowebrtc)
example.com#?#.item:has(.ad)
##.ok";
        let c = convert(list.lines());
        assert_eq!(c.rules, 1, "only the plain cosmetic rule survives");
        assert_eq!(c.skipped, 5);
    }

    #[test]
    fn comments_and_headers_ignored() {
        let c = convert("[Adblock Plus 2.0]\n! comment\n".lines());
        assert_eq!(c.rules, 0);
        assert_eq!(c.skipped, 0);
    }

    #[test]
    fn bare_star_skipped_but_scoped_star_kept() {
        let c = convert("*\n".lines());
        assert_eq!(c.rules, 0);
        let r = one("*$script,domain=ads.com");
        assert_eq!(r["trigger"]["resource-type"], serde_json::json!(["script"]));
    }

    #[test]
    fn popup_and_match_case() {
        let r = one("||popads.net^$popup,match-case");
        assert_eq!(r["trigger"]["resource-type"], serde_json::json!(["popup"]));
        assert_eq!(r["trigger"]["url-filter-is-case-sensitive"], true);
    }

    #[test]
    fn option_only_rule_gets_match_all_filter() {
        // `$popup,domain=...` has no address pattern; url-filter must
        // still be nonempty or WebKit rejects the whole ruleset.
        let r = one("$popup,domain=annoying.com");
        assert_eq!(r["trigger"]["url-filter"], ".*");
        assert_eq!(r["trigger"]["resource-type"], serde_json::json!(["popup"]));
    }

    #[test]
    fn patternless_broadened_type_skipped() {
        // `$ping,third-party` widened to "raw" would block all
        // third-party XHR. Must be dropped, not approximated.
        let c = convert("$ping,third-party\n".lines());
        assert_eq!(c.rules, 0);
        assert_eq!(c.skipped, 1);
        // With a URL pattern the widening is safely narrowed. Kept.
        let r = one("||tracker.com^$ping");
        assert_eq!(r["trigger"]["resource-type"], serde_json::json!(["raw"]));
    }
}
