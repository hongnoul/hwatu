// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Justin Hong
//! Extension-parity coverage verbs (roadmap C2-C5): the actions a
//! focus-stealing in-browser extension bridge offers, rebuilt on the
//! headless daemon. Everything here composes the existing automation
//! primitives (eval machinery, window pool, profiles); nothing maps a
//! window or requests focus.

use crate::automation::{self, eval_with, js_string, json_or_null, resolve, NavPolicy, Reply};
use crate::window::BrowserWindow;
use crate::Daemon;
use hwatu_ipc::{ContentFormat, FormField, OpenMode, Request, Response, FORK_MAX_COUNT};
use std::cell::RefCell;
use std::rc::Rc;

// ---- C2: get_content -------------------------------------------------

/// Shared with every engine backend: `page-scripts/content.js`.
pub(crate) const CONTENT_JS: &str = include_str!("../../../page-scripts/content.js");

// Args mirror Request::GetContent field-for-field: this is a verb handler
// unpacking a protocol message, not a general-purpose API to slim down.
#[allow(clippy::too_many_arguments)]
pub fn get_content(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
    format: ContentFormat,
    selector: Option<String>,
    nth: Option<u32>,
    contains: Option<String>,
    max_chars: Option<usize>,
    timeout_ms: Option<u64>,
    reply: Reply,
) {
    let max_chars = max_chars.unwrap_or(64 * 1024).clamp(1, 4 * 1024 * 1024);
    let scope = match selector.as_deref() {
        Some(sel) => {
            let prelude =
                match automation::target_prelude(Some(sel), nth, contains.as_deref(), None) {
                    Ok(p) => p,
                    Err(resp) => return automation::send_once(reply, *resp),
                };
            format!("{prelude}\nconst root = el;")
        }
        None => "const root = document.documentElement;".to_string(),
    };
    let format = match format {
        ContentFormat::Text => "text",
        ContentFormat::Html => "html",
        ContentFormat::Title => "title",
        ContentFormat::Links => "links",
    };
    let js = format!("const MAX = {max_chars};\nconst FORMAT = '{format}';\n{scope}\n{CONTENT_JS}");
    eval_with(daemon, id, js, timeout_ms, NavPolicy::Error, reply);
}

// ---- C2: fill_form ---------------------------------------------------

/// Shared with every engine backend: `page-scripts/fill.js`.
pub(crate) const FILL_JS: &str = include_str!("../../../page-scripts/fill.js");

pub fn fill_form(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
    fields: Vec<FormField>,
    submit: bool,
    timeout_ms: Option<u64>,
    reply: Reply,
) {
    if fields.is_empty() {
        return automation::send_once(reply, Response::err("fill_form needs at least one field"));
    }
    if fields.len() > hwatu_ipc::FILL_FORM_MAX_FIELDS {
        return automation::send_once(
            reply,
            Response::err(format!(
                "fill_form has {} fields; the cap is {}",
                fields.len(),
                hwatu_ipc::FILL_FORM_MAX_FIELDS
            )),
        );
    }
    let mut specs = Vec::with_capacity(fields.len());
    for (i, f) in fields.iter().enumerate() {
        if f.selector.is_some() == f.r#ref.is_some() {
            return automation::send_once(
                reply,
                Response::err(format!(
                    "fill_form field {i}: pass exactly one of selector or ref"
                )),
            );
        }
        if f.value.is_some() == f.checked.is_some() {
            return automation::send_once(
                reply,
                Response::err(format!(
                    "fill_form field {i}: pass exactly one of value or checked"
                )),
            );
        }
        specs.push(format!(
            r#"{{ selector: {selector}, nth: {nth}, contains: {contains}, ref: {r}, value: {value}, checked: {checked} }}"#,
            selector = json_or_null(f.selector.as_deref()),
            nth = f.nth.unwrap_or(0),
            contains = json_or_null(f.contains.as_deref()),
            r = f.r#ref.map_or("null".into(), |v| v.to_string()),
            value = json_or_null(f.value.as_deref()),
            checked = f.checked.map_or("null".into(), |v| v.to_string()),
        ));
    }
    let js = format!(
        "{native}\nconst FIELDS = [{fields}];\nconst SUBMIT = {submit};\n{FILL_JS}",
        native = crate::automation::NATIVE_TIME_JS,
        fields = specs.join(", "),
        submit = submit,
    );
    // Submit may navigate; treat that as success like Type's enter.
    let nav = if submit {
        NavPolicy::Success
    } else {
        NavPolicy::Error
    };
    eval_with(daemon, id, js, timeout_ms, nav, reply);
}

// ---- C5: drop_file ---------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub fn drop_file(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
    selector: String,
    nth: Option<u32>,
    contains: Option<String>,
    path: Option<String>,
    data: Option<String>,
    name: Option<String>,
    mime: Option<String>,
    timeout_ms: Option<u64>,
    reply: Reply,
) {
    let bytes = if let Some(encoded) = data {
        let max_encoded = hwatu_ipc::INLINE_MAX_BYTES.div_ceil(3) * 4;
        if encoded.len() > max_encoded {
            return automation::send_once(
                reply,
                Response::err(format!(
                    "drop_file data exceeds the {}-byte decoded limit",
                    hwatu_ipc::INLINE_MAX_BYTES
                )),
            );
        }
        match hwatu_ipc::base64::decode(&encoded) {
            Ok(bytes) => bytes,
            Err(error) => {
                return automation::send_once(
                    reply,
                    Response::err(format!("drop_file data is not valid base64: {error}")),
                );
            }
        }
    } else if let Some(path) = path.as_deref() {
        match std::fs::read(path) {
            Ok(bytes) if bytes.len() <= hwatu_ipc::INLINE_MAX_BYTES => bytes,
            Ok(bytes) => {
                return automation::send_once(
                    reply,
                    Response::err(format!(
                        "{path} is {} bytes; drop_file caps at {}",
                        bytes.len(),
                        hwatu_ipc::INLINE_MAX_BYTES
                    )),
                );
            }
            Err(e) => {
                return automation::send_once(
                    reply,
                    Response::err(format!("cannot read {path}: {e}")),
                );
            }
        }
    } else {
        return automation::send_once(reply, Response::err("drop_file needs path or data"));
    };
    let file_name = name
        .or_else(|| {
            path.as_deref().and_then(|p| {
                std::path::Path::new(p)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
            })
        })
        .unwrap_or_else(|| "file".to_string());
    let mime = mime.unwrap_or_else(|| guess_mime(&file_name).to_string());
    let prelude = match automation::target_prelude(Some(&selector), nth, contains.as_deref(), None)
    {
        Ok(p) => p,
        Err(resp) => return automation::send_once(reply, *resp),
    };
    let js = format!(
        r#"{prelude}
const B64 = {b64};
const NAME = {name};
const MIME = {mime};
const binary = atob(B64);
const bytes = new Uint8Array(binary.length);
for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
const file = new File([bytes], NAME, {{ type: MIME }});
const dt = new DataTransfer();
dt.items.add(file);
el.scrollIntoView({{ block: 'center', behavior: 'instant' }});
const rect = el.getBoundingClientRect();
const opts = {{ bubbles: true, cancelable: true, dataTransfer: dt,
               clientX: rect.left + rect.width / 2, clientY: rect.top + rect.height / 2 }};
el.dispatchEvent(new DragEvent('dragenter', opts));
el.dispatchEvent(new DragEvent('dragover', opts));
const accepted = el.dispatchEvent(new DragEvent('drop', opts));
// Drop zones wrapping a hidden <input type=file> often only listen on
// the input; feed it too so both wiring styles work.
const input = el.matches('input[type=file]') ? el : el.querySelector('input[type=file]');
if (input) {{
  const inputDt = new DataTransfer();
  inputDt.items.add(file);
  input.files = inputDt.files;
  input.dispatchEvent(new Event('input', {{ bubbles: true }}));
  input.dispatchEvent(new Event('change', {{ bubbles: true }}));
}}
return {{ dropped: matched, name: NAME, mime: MIME, bytes: bytes.length,
          defaultPrevented: !accepted, fedInput: !!input, url: location.href }};"#,
        b64 = js_string(&hwatu_ipc::base64::encode(&bytes)),
        name = js_string(&file_name),
        mime = js_string(&mime),
    );
    eval_with(daemon, id, js, timeout_ms, NavPolicy::Error, reply);
}

fn guess_mime(name: &str) -> &'static str {
    match name
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
    {
        Some(ext) => match ext.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "svg" => "image/svg+xml",
            "pdf" => "application/pdf",
            "txt" | "md" => "text/plain",
            "csv" => "text/csv",
            "json" => "application/json",
            "zip" => "application/zip",
            "html" | "htm" => "text/html",
            _ => "application/octet-stream",
        },
        None => "application/octet-stream",
    }
}

// ---- C5: auth_context ------------------------------------------------

/// Shared with every engine backend: `page-scripts/auth-context.js`.
pub(crate) const AUTH_CONTEXT_JS: &str = include_str!("../../../page-scripts/auth-context.js");

pub fn auth_context(daemon: &Rc<Daemon>, id: Option<u64>, reply: Reply) {
    // Cookie inventory without values: names, session vs persistent,
    // and expiry horizon come from document.cookie's names plus the
    // Cookie Store API when available; localStorage keys likewise.
    // Values never leave the page.
    const JS: &str = AUTH_CONTEXT_JS;
    let win = match resolve(daemon, id) {
        Ok(win) => win,
        Err(resp) => return automation::send_once(reply, *resp),
    };
    let profile = win.profile.borrow().clone();
    let reply: Reply = Box::new(move |response| {
        // Annotate with the daemon-side profile name so the agent
        // knows which session store answered.
        let response = match response {
            Response::Ok {
                value: Some(mut value),
                window,
                windows,
                adblock,
                path,
                data,
            } => {
                if let Some(map) = value.as_object_mut() {
                    map.insert(
                        "profile".into(),
                        profile
                            .clone()
                            .map(serde_json::Value::from)
                            .unwrap_or(serde_json::Value::Null),
                    );
                    // Structured stop: a visible login wall becomes
                    // `login_required` so the agent asks the user.
                    let wall = map.remove("login_wall").filter(|v| !v.is_null());
                    let url = map
                        .get("url")
                        .and_then(|u| u.as_str())
                        .unwrap_or("")
                        .to_string();
                    map.insert(
                        "login_required".into(),
                        wall.map(|w| crate::stops::login_required(&w, &url))
                            .unwrap_or(serde_json::Value::Null),
                    );
                }
                Response::Ok {
                    value: Some(value),
                    window,
                    windows,
                    adblock,
                    path,
                    data,
                }
            }
            other => other,
        };
        reply(response);
    });
    eval_with(
        daemon,
        Some(win.id),
        JS.replace("__LOGIN_WALL__", crate::stops::LOGIN_WALL_JS.trim()),
        None,
        NavPolicy::Error,
        reply,
    );
}

// ---- C3: fork / list_forks -------------------------------------------

pub fn fork(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
    name: Option<String>,
    count: Option<u32>,
    timeout_ms: Option<u64>,
    reply: Reply,
) {
    let count = count.unwrap_or(1);
    if count == 0 || count > FORK_MAX_COUNT {
        return automation::send_once(
            reply,
            Response::err(format!("fork count must be 1..={FORK_MAX_COUNT}")),
        );
    }
    let source = match resolve(daemon, id) {
        Ok(win) => win,
        Err(resp) => return automation::send_once(reply, *resp),
    };
    let url = source.info().url;
    if url.is_empty() || url == "about:blank" {
        return automation::send_once(
            reply,
            Response::err("fork source has no page loaded; navigate it first"),
        );
    }
    let profile = source.profile.borrow().clone();
    let parent_id = source.id;

    let mut forks = Vec::new();
    for i in 0..count {
        let fork_name = match (&name, count) {
            (Some(n), 1) => n.clone(),
            (Some(n), _) => format!("{n}-{}", i + 1),
            (None, _) => format!("fork-{parent_id}-{}", next_fork_seq()),
        };
        let info = BrowserWindow::open_with_profile(
            daemon,
            Some(url.clone()),
            None,
            OpenMode::Headless,
            profile.clone(),
        );
        if let Some(win) = daemon.windows.borrow().get(&info.id) {
            win.fork_of.replace(Some((
                parent_id,
                fork_name.clone(),
                std::time::SystemTime::now(),
            )));
        }
        forks.push(serde_json::json!({
            "id": info.id,
            "name": fork_name,
            "parent": parent_id,
            "url": url,
        }));
    }
    // Wait for the first fork's load so a follow-up eval targets real
    // content; the rest load in parallel behind it.
    let first_id = forks[0]["id"].as_u64().unwrap();
    let value = serde_json::json!({ "forks": forks });
    let reply_once = automation::once(reply);
    let send = {
        let reply_once = reply_once.clone();
        move || {
            reply_once(Response::value(value));
        }
    };
    automation::after_load(daemon, first_id, timeout_ms, Box::new(send));
}

static FORK_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn next_fork_seq() -> u64 {
    FORK_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1
}

pub fn list_forks(daemon: &Rc<Daemon>) -> Response {
    let windows = daemon.windows.borrow();
    let mut forks: Vec<serde_json::Value> = Vec::new();
    for win in windows.values() {
        if let Some((parent, name, created)) = win.fork_of.borrow().clone() {
            forks.push(serde_json::json!({
                "id": win.id,
                "name": name,
                "parent": parent,
                "parent_open": windows.contains_key(&parent),
                "url": win.info().url,
                "created_at": created
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
            }));
        }
    }
    forks.sort_by_key(|f| f["id"].as_u64());
    Response::value(serde_json::json!({ "forks": forks }))
}

// ---- C3: try_until ---------------------------------------------------

pub fn try_until(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
    alternatives: Vec<Request>,
    timeout_ms: Option<u64>,
    reply: Reply,
) {
    if let Err(message) = Request::validate_try_until(&alternatives) {
        return automation::send_once(reply, Response::err(message));
    }
    let state = Rc::new(TryUntilState {
        daemon: daemon.clone(),
        id,
        alternatives: RefCell::new(alternatives.into_iter().enumerate().collect()),
        errors: RefCell::new(Vec::new()),
        deadline: std::time::Instant::now()
            + std::time::Duration::from_millis(timeout_ms.unwrap_or(5000).max(1)),
        reply: automation::once(reply),
    });
    try_until_step(state);
}

struct TryUntilState {
    daemon: Rc<Daemon>,
    id: Option<u64>,
    alternatives: RefCell<std::collections::VecDeque<(usize, Request)>>,
    errors: RefCell<Vec<serde_json::Value>>,
    deadline: std::time::Instant,
    reply: Rc<dyn Fn(Response)>,
}

fn try_until_step(state: Rc<TryUntilState>) {
    let Some((index, alt)) = state.alternatives.borrow_mut().pop_front() else {
        let errors = state.errors.borrow().clone();
        (state.reply)(Response::value(serde_json::json!({
            "success": false,
            "errors": errors,
            "message": "all alternatives failed",
        })));
        return;
    };
    if std::time::Instant::now() >= state.deadline {
        let errors = state.errors.borrow().clone();
        (state.reply)(Response::value(serde_json::json!({
            "success": false,
            "errors": errors,
            "message": "try_until deadline reached",
        })));
        return;
    }
    let remaining = state
        .deadline
        .saturating_duration_since(std::time::Instant::now())
        .as_millis() as u64;
    // Individual alternatives get a short slice of the overall budget
    // so one hung selector cannot eat every other candidate's chance.
    let per_step = remaining.clamp(1, 2000);
    let kind = alt.kind();
    let state2 = state.clone();
    let step_reply: Reply = Box::new(move |response| match response {
        Response::Ok { value, .. } => {
            (state2.reply)(Response::value(serde_json::json!({
                "success": true,
                "alternative": index,
                "action": kind,
                "result": value,
            })));
        }
        Response::Err { message, .. } => {
            state2.errors.borrow_mut().push(serde_json::json!({
                "alternative": index,
                "action": kind,
                "error": message,
            }));
            try_until_step(state2.clone());
        }
    });
    dispatch_alternative(&state.daemon, state.id, alt, per_step, step_reply);
}

fn dispatch_alternative(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
    alt: Request,
    timeout_ms: u64,
    reply: Reply,
) {
    match alt {
        Request::Click {
            id: alt_id,
            selector,
            nth,
            contains,
            r#ref,
            trusted,
            confirm,
            ..
        } => automation::click(
            daemon,
            alt_id.or(id),
            selector,
            nth,
            contains,
            r#ref,
            trusted,
            confirm,
            Some(timeout_ms),
            reply,
        ),
        Request::Type {
            id: alt_id,
            selector,
            nth,
            contains,
            r#ref,
            text,
            trusted,
            clear,
            enter,
            ..
        } => automation::type_text(
            daemon,
            alt_id.or(id),
            selector,
            nth,
            contains,
            r#ref,
            text,
            trusted,
            clear,
            enter,
            Some(timeout_ms),
            reply,
        ),
        Request::Expect {
            id: alt_id,
            selector,
            nth,
            contains,
            text,
            absent,
            visible,
            ..
        } => automation::expect(
            daemon,
            alt_id.or(id),
            selector,
            nth,
            contains,
            text,
            absent,
            visible,
            Some(timeout_ms),
            reply,
        ),
        other => automation::send_once(
            reply,
            Response::err(format!("try_until cannot run {}", other.kind())),
        ),
    }
}

// ---- C4: scout -------------------------------------------------------

const SCOUT_MAX_DEPTH: u32 = 2;
const SCOUT_MAX_PAGES: u32 = 10;

#[allow(clippy::too_many_arguments)]
pub fn scout(
    daemon: &Rc<Daemon>,
    url: String,
    depth: Option<u32>,
    max_pages: Option<u32>,
    filter: Option<String>,
    budget: Option<usize>,
    profile: Option<String>,
    timeout_ms: Option<u64>,
    reply: Reply,
) {
    let depth = depth.unwrap_or(1).min(SCOUT_MAX_DEPTH);
    let max_pages = max_pages.unwrap_or(5).clamp(1, SCOUT_MAX_PAGES);
    let budget = budget.unwrap_or(2000).clamp(200, 20_000);
    let host = match url::host_of(&url) {
        Some(h) => h,
        None => {
            return automation::send_once(
                reply,
                Response::err(format!("scout needs an absolute http(s) URL, got {url}")),
            )
        }
    };
    // One headless window for the whole crawl: warm engine, shared
    // session, sequential loads. Closed when the crawl ends.
    let info = BrowserWindow::open_with_profile(
        daemon,
        Some(url.clone()),
        None,
        OpenMode::Headless,
        profile,
    );
    let state = Rc::new(ScoutState {
        daemon: daemon.clone(),
        window: info.id,
        host,
        filter,
        budget,
        max_pages,
        deadline: std::time::Instant::now()
            + std::time::Duration::from_millis(timeout_ms.unwrap_or(60_000).max(1)),
        queue: RefCell::new(std::collections::VecDeque::from([(url, 0u32)])),
        visited: RefCell::new(std::collections::HashSet::new()),
        pages: RefCell::new(Vec::new()),
        depth,
        navigated_first: std::cell::Cell::new(true),
        reply: automation::once(reply),
    });
    scout_step(state);
}

struct ScoutState {
    daemon: Rc<Daemon>,
    window: u64,
    host: String,
    filter: Option<String>,
    budget: usize,
    max_pages: u32,
    depth: u32,
    deadline: std::time::Instant,
    queue: RefCell<std::collections::VecDeque<(String, u32)>>,
    visited: RefCell<std::collections::HashSet<String>>,
    pages: RefCell<Vec<serde_json::Value>>,
    /// The window's opening navigation already targets the first URL;
    /// later URLs need an explicit navigate.
    navigated_first: std::cell::Cell<bool>,
    reply: Rc<dyn Fn(Response)>,
}

fn scout_finish(state: &Rc<ScoutState>) {
    let pages = state.pages.borrow().clone();
    // Close the crawl window; the reply does not depend on it. The
    // borrow must end before close(): the destroy handler re-enters
    // daemon.windows with a mut borrow.
    let win = state.daemon.windows.borrow().get(&state.window).cloned();
    if let Some(win) = win {
        win.close();
    }
    (state.reply)(Response::value(serde_json::json!({
        "pages": pages,
        "visited": state.visited.borrow().len(),
    })));
}

fn scout_step(state: Rc<ScoutState>) {
    if state.pages.borrow().len() as u32 >= state.max_pages
        || std::time::Instant::now() >= state.deadline
    {
        return scout_finish(&state);
    }
    let Some((url, level)) = state.queue.borrow_mut().pop_front() else {
        return scout_finish(&state);
    };
    if !state.visited.borrow_mut().insert(url.clone()) {
        return scout_step(state);
    }
    let first = state.navigated_first.replace(false);
    let after_nav = {
        let state = state.clone();
        let url = url.clone();
        move |ok: bool| {
            if !ok {
                state.pages.borrow_mut().push(serde_json::json!({
                    "url": url,
                    "error": "load failed or timed out",
                }));
                return scout_step(state);
            }
            scout_harvest(state, url, level);
        }
    };
    if first {
        // Window is already loading the first URL from open().
        let state2 = state.clone();
        automation::after_load(
            &state2.daemon.clone(),
            state2.window,
            Some(remaining_ms(&state2, 15_000)),
            Box::new(move || after_nav(true)),
        );
    } else {
        let reply: Reply = Box::new(move |response| {
            after_nav(!matches!(response, Response::Err { .. }));
        });
        automation::navigate(
            &state.daemon.clone(),
            Some(state.window),
            url,
            true,
            hwatu_ipc::LoadStage::Settled,
            Some(remaining_ms(&state, 15_000)),
            reply,
        );
    }
}

fn remaining_ms(state: &ScoutState, cap: u64) -> u64 {
    state
        .deadline
        .saturating_duration_since(std::time::Instant::now())
        .as_millis()
        .min(cap as u128)
        .max(1) as u64
}

fn scout_harvest(state: Rc<ScoutState>, url: String, level: u32) {
    let js = format!(
        r#"
const BUDGET = {budget};
const text = (document.body ? document.body.innerText : '')
  .replace(/[ \t]+/g, ' ').replace(/\n{{3,}}/g, '\n\n').trim();
const seen = new Set();
const links = [];
for (const a of document.querySelectorAll('a[href]')) {{
  if (links.length >= 40) break;
  const href = a.href;
  if (!href || href.startsWith('javascript:') || seen.has(href)) continue;
  const t = (a.innerText || a.getAttribute('aria-label') || '').replace(/\s+/g, ' ').trim().slice(0, 80);
  if (!t || t.length < 2) continue;
  seen.add(href);
  links.push({{ url: href, text: t }});
}}
return {{ url: location.href, title: document.title,
          text: text.slice(0, BUDGET), truncated: text.length > BUDGET, links }};"#,
        budget = state.budget,
    );
    let reply: Reply = Box::new({
        let state = state.clone();
        move |response| {
            match response {
                Response::Ok {
                    value: Some(value), ..
                } => {
                    // Queue same-host links for the next level.
                    if level < state.depth {
                        if let Some(links) = value.get("links").and_then(|l| l.as_array()) {
                            let mut queue = state.queue.borrow_mut();
                            for link in links {
                                let (Some(href), text) = (
                                    link.get("url").and_then(|u| u.as_str()),
                                    link.get("text").and_then(|t| t.as_str()).unwrap_or(""),
                                ) else {
                                    continue;
                                };
                                if url::host_of(href).as_deref() != Some(state.host.as_str()) {
                                    continue;
                                }
                                if let Some(f) = &state.filter {
                                    let f = f.to_lowercase();
                                    if !href.to_lowercase().contains(&f)
                                        && !text.to_lowercase().contains(&f)
                                    {
                                        continue;
                                    }
                                }
                                if state.visited.borrow().contains(href) {
                                    continue;
                                }
                                queue.push_back((href.to_string(), level + 1));
                            }
                        }
                    }
                    let mut page = value;
                    if let Some(map) = page.as_object_mut() {
                        map.insert("depth".into(), level.into());
                    }
                    state.pages.borrow_mut().push(page);
                }
                Response::Ok { .. } => {
                    state
                        .pages
                        .borrow_mut()
                        .push(serde_json::json!({ "url": url, "error": "empty harvest" }));
                }
                Response::Err { message, .. } => {
                    state
                        .pages
                        .borrow_mut()
                        .push(serde_json::json!({ "url": url, "error": message }));
                }
            }
            scout_step(state.clone());
        }
    });
    eval_with(
        &state.daemon.clone(),
        Some(state.window),
        js,
        Some(remaining_ms(&state, 10_000)),
        NavPolicy::Error,
        reply,
    );
}

/// Tiny host extractor; scout's same-host rule needs nothing more.
mod url {
    pub fn host_of(url: &str) -> Option<String> {
        let rest = url
            .strip_prefix("https://")
            .or_else(|| url.strip_prefix("http://"))?;
        let host = rest.split(['/', '?', '#']).next()?;
        let host = host.split('@').next_back()?.split(':').next()?;
        if host.is_empty() {
            return None;
        }
        Some(host.trim_start_matches("www.").to_ascii_lowercase())
    }
}

// ---- C6: fill_login --------------------------------------------------

// ---- C1: list_frames -------------------------------------------------

/// Enumerate the frame tree from the top document. Same-origin frames
/// report their document url/title and are reachable by every
/// selector-based verb (target_prelude traverses them); cross-origin
/// frames are listed with `accessible: false`. WebKitGTK's embedder
/// API has no public cross-origin script entry; claiming otherwise
/// would be a lie, so the honest capability report is the feature.
pub fn list_frames(daemon: &Rc<Daemon>, id: Option<u64>, timeout_ms: Option<u64>, reply: Reply) {
    const JS: &str = r#"
const frames = [];
const walk = (doc, parent, depth) => {
  if (depth > 5) return;
  for (const el of doc.querySelectorAll('iframe,frame')) {
    const entry = {
      index: frames.length,
      parent,
      tag: el.localName,
      name: el.name || null,
      src: el.src || null,
      rect: (() => { const r = el.getBoundingClientRect(); return [r.x, r.y, r.width, r.height]; })(),
    };
    let child = null;
    try { child = el.contentDocument; } catch (_) {}
    if (child) {
      entry.accessible = true;
      entry.url = child.location ? child.location.href : null;
      entry.title = child.title || null;
      frames.push(entry);
      walk(child, entry.index, depth + 1);
    } else {
      entry.accessible = false;
      entry.origin = (() => { try { return new URL(el.src).origin; } catch (_) { return null; } })();
      frames.push(entry);
    }
  }
};
walk(document, null, 0);
return { url: location.href, frames,
         note: frames.some(f => !f.accessible)
           ? 'cross-origin frames are listed but not scriptable from the embedder; selector verbs reach same-origin frames automatically'
           : undefined };"#;
    eval_with(
        daemon,
        id,
        JS.to_string(),
        timeout_ms,
        NavPolicy::Error,
        reply,
    );
}

/// Agent-facing password/TOTP fill (coverage C6). Same backends and
/// page JS as `passfill.rs`, but addressable over
/// IPC and honest in its reply. Secrets go page-ward only.
pub fn fill_login(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
    kind: hwatu_ipc::LoginFill,
    host_override: Option<String>,
    timeout_ms: Option<u64>,
    reply: Reply,
) {
    let win = match resolve(daemon, id) {
        Ok(win) => win,
        Err(resp) => return automation::send_once(reply, *resp),
    };
    let host = host_override
        .or_else(|| url::host_of(&win.info().url))
        .unwrap_or_default();
    if host.is_empty() || host.starts_with("hwatu") {
        return automation::send_once(
            reply,
            Response::err("no site to fill (window has no http(s) page)"),
        );
    }
    // Blocking store lookup on a worker thread (gpg pinentry can take
    // seconds; the GTK loop must not block), then the fill JS on the
    // main loop. Mirrors window.rs's fill_password wiring.
    let (tx, rx) = std::sync::mpsc::channel();
    {
        let host = host.clone();
        std::thread::spawn(move || {
            let _ = tx.send(match kind {
                hwatu_ipc::LoginFill::Password => crate::passfill::lookup(&host)
                    .map(|credential| crate::passfill::fill_js(&credential)),
                hwatu_ipc::LoginFill::Otp => crate::passfill::lookup_otp(&host)
                    .map(|code| crate::passfill::fill_otp_js(&code)),
            });
        });
    }
    let daemon = daemon.clone();
    let win_id = win.id;
    let reply = automation::once(reply);
    let deadline = std::time::Instant::now()
        + std::time::Duration::from_millis(timeout_ms.unwrap_or(30_000).max(1));
    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        match rx.try_recv() {
            Ok(Ok(js)) => {
                let reply = reply.clone();
                let step: Reply = Box::new(move |response| reply(response));
                eval_with(&daemon, Some(win_id), js, None, NavPolicy::Success, step);
                glib::ControlFlow::Break
            }
            Ok(Err(error)) => {
                reply(Response::err(error.to_string()));
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                if std::time::Instant::now() >= deadline {
                    reply(Response::err("credential lookup timed out"));
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                reply(Response::err("credential lookup crashed"));
                glib::ControlFlow::Break
            }
        }
    });
}
