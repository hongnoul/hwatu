// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! Agent automation IPC: eval / navigate / screenshot / wait_load.
//!
//! These requests are asynchronous on the GTK main loop (JS evaluation,
//! page loads, snapshot encoding), so unlike the rest of the protocol
//! they take a deferred `Reply` instead of returning a `Response`.
//! Built for coding agents (jcode et al.) that verify web UIs: run JS
//! in the page, wait for loads, capture pixels.

use crate::window::BrowserWindow;
use crate::Daemon;
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use hwatu_ipc::{LoadStage, OpenMode, Response};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use webkit6::prelude::*;

/// Deferred response writer, callable exactly once.
pub type Reply = Box<dyn FnOnce(Response)>;

/// A `Reply` that several racing callbacks (signal, timeout, cancel)
/// can share; only the first `send` wins.
#[derive(Clone)]
struct OnceReply(Rc<RefCell<Option<Reply>>>);

impl OnceReply {
    fn new(reply: Reply) -> Self {
        Self(Rc::new(RefCell::new(Some(reply))))
    }
    fn send(&self, response: Response) {
        if let Some(reply) = self.0.borrow_mut().take() {
            reply(response);
        }
    }
    fn is_spent(&self) -> bool {
        self.0.borrow().is_none()
    }
}

/// Default deadline for eval / navigate / wait_load.
const DEFAULT_TIMEOUT_MS: u64 = 15_000;

/// JS prelude giving harness scripts clocks that keep running while
/// the page's virtual clock (crate::clock) is paused. Falls back to
/// the page globals when the clock script is absent (old pages loaded
/// before a daemon upgrade).
const NATIVE_TIME_JS: &str = r#"
const __hwatuNative = (window.__hwatu_clock && window.__hwatu_clock.native) || null;
const hwatuSleep = (ms) => new Promise((r) =>
  __hwatuNative ? __hwatuNative.setTimeout(r, ms) : setTimeout(r, ms));
const hwatuNow = () => (__hwatuNative ? __hwatuNative.dateNow() : Date.now());
"#;

/// Pick the target window: explicit id, else the focused window, else
/// the last window an automation command targeted (if still open),
/// else the only window. Every successful resolution records the
/// target, so an agent that opened or drove a window keeps addressing
/// it without repeating `id` even when nothing has WM focus (the
/// normal state for background/headless verification flows).
/// Genuine ambiguity is still an error rather than a guess: an agent
/// driving the wrong window is worse than a retry with an id.
fn resolve(daemon: &Rc<Daemon>, id: Option<u64>) -> Result<Rc<BrowserWindow>, Box<Response>> {
    let win = resolve_uncached(daemon, id)?;
    daemon.last_target.replace(Some(win.id));
    Ok(win)
}

fn resolve_uncached(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
) -> Result<Rc<BrowserWindow>, Box<Response>> {
    let windows = daemon.windows.borrow();
    if let Some(id) = id {
        return windows
            .get(&id)
            .cloned()
            .ok_or_else(|| Box::new(Response::err(format!("no window {id}"))));
    }
    if let Some(focused) = windows.values().find(|w| w.window.is_active()) {
        return Ok(focused.clone());
    }
    if let Some(last) = *daemon.last_target.borrow() {
        if let Some(win) = windows.get(&last) {
            return Ok(win.clone());
        }
    }
    match windows.len() {
        0 => Err(Box::new(Response::err("no windows open"))),
        1 => Ok(windows.values().next().cloned().expect("len checked")),
        n => Err(Box::new(Response::err(format!(
            "{n} windows open and none focused; pass an explicit id (see `hwatu list`)"
        )))),
    }
}

/// Live WebView of a window, reviving it from a discard first and
/// re-asserting the offscreen viewport for headless windows (GTK can
/// re-allocate an unmapped toplevel to 0x0 behind our back).
fn live_view(win: &Rc<BrowserWindow>) -> Result<webkit6::WebView, Box<Response>> {
    win.restore();
    win.ensure_viewport();
    win.live_webview()
        .ok_or_else(|| Box::new(Response::err("window has no live webview")))
}

/// Convert a JS evaluation result to JSON. `undefined` maps to null;
/// values JSON can't express (functions, symbols) fall back to their
/// string form.
fn jsc_to_json(value: &webkit6::javascriptcore::Value) -> serde_json::Value {
    if value.is_undefined() || value.is_null() {
        return serde_json::Value::Null;
    }
    value
        .to_json(0)
        .and_then(|s| serde_json::from_str(s.as_str()).ok())
        .unwrap_or_else(|| serde_json::Value::String(value.to_str().to_string()))
}

/// Arm a shared timeout that sends an error if it fires before the
/// winning callback. Fired/late sources clean themselves up.
fn arm_timeout(reply: OnceReply, timeout_ms: Option<u64>, what: &'static str) {
    let ms = timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
    if ms == 0 {
        return; // explicit 0 disables the deadline
    }
    glib::timeout_add_local_once(std::time::Duration::from_millis(ms), move || {
        reply.send(Response::err(format!("{what} timed out after {ms} ms")));
    });
}

/// What a cross-document navigation during an eval means for the
/// caller. The page committing a new document destroys the running
/// script's JS context, so the eval can never resolve normally; the
/// question is whether that navigation is a failure or the point.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NavPolicy {
    /// Navigation is an interruption (plain `eval`, `snapshot`,
    /// `scroll`): reply with an error naming the destination, so the
    /// agent re-targets the new document instead of reading `null`
    /// or waiting out the full deadline.
    Error,
    /// Navigation is the expected outcome (`click` on a link, `type
    /// --enter` submitting a form, `challenge --wait` where solving
    /// reloads the page): wait for the load to finish, then reply
    /// `{navigated: true, url}` as a success.
    Success,
}

/// Run `js` in the page: as an *expression* when it parses as one (so
/// `document.title` just works, the way every agent harness expects),
/// else as an async *function body* (so `const x = ...; return x`
/// also works). The choice is made by a compile-only probe that
/// defines but never calls a function wrapping the expression, so
/// user code runs exactly once regardless of form. A returned Promise
/// is awaited by WebKit before the callback fires.
///
/// If the page navigates while the script runs (a click handler that
/// follows a link, `location =`, a form submit), the document's JS
/// context is destroyed and WebKit never fires the completion
/// callback: without intervention the caller sees a silent `null` or
/// a full deadline timeout. A load watcher resolves the reply per
/// [`NavPolicy`] instead.
pub fn eval(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
    js: String,
    timeout_ms: Option<u64>,
    reply: Reply,
) {
    eval_with(daemon, id, js, timeout_ms, NavPolicy::Error, reply)
}

pub fn eval_with(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
    js: String,
    timeout_ms: Option<u64>,
    nav: NavPolicy,
    reply: Reply,
) {
    let reply = OnceReply::new(reply);
    let view = match resolve(daemon, id).and_then(|w| live_view(&w)) {
        Ok(v) => v,
        Err(resp) => return reply.send(*resp),
    };

    let cancellable = gio::Cancellable::new();
    // The deadline also cancels the evaluation so a hung Promise does
    // not pin the callback (and its captured connection) forever.
    {
        let reply = reply.clone();
        let cancellable = cancellable.clone();
        let ms = timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
        if ms > 0 {
            glib::timeout_add_local_once(std::time::Duration::from_millis(ms), move || {
                if !reply.is_spent() {
                    cancellable.cancel();
                    reply.send(Response::err(format!("eval timed out after {ms} ms")));
                }
            });
        }
    }

    // Cross-document navigation destroys the JS context mid-eval.
    // `Committed` is the moment the new document replaces the old one
    // (same-document changes like pushState/hash never commit), so it
    // is exactly when the pending completion callback becomes dead.
    let nav_watch: Rc<RefCell<Option<glib::SignalHandlerId>>> = Rc::new(RefCell::new(None));
    {
        let reply = reply.clone();
        let cancellable = cancellable.clone();
        let nav_watch2 = nav_watch.clone();
        let handler = view.connect_load_changed(move |view, event| {
            if event != webkit6::LoadEvent::Committed {
                return;
            }
            if let Some(id) = nav_watch2.borrow_mut().take() {
                view.disconnect(id);
            }
            if reply.is_spent() {
                return;
            }
            cancellable.cancel();
            let uri = view.uri().map(|u| u.to_string()).unwrap_or_default();
            match nav {
                NavPolicy::Error => reply.send(Response::err(format!(
                    "eval interrupted: the page navigated to {uri} while the script \
                     was running, destroying its JS context. Wait for the load \
                     (`hwatu wait-load`) and run the script against the new document."
                ))),
                NavPolicy::Success => {
                    // The action triggered the navigation it was meant
                    // to; hold the reply until the destination finishes
                    // loading so a follow-up snapshot sees real content.
                    let reply = reply.clone();
                    wire_load_finished(view, move |view| {
                        let url = view.uri().map(|u| u.to_string()).unwrap_or(uri);
                        reply.send(Response::value(serde_json::json!({
                            "navigated": true,
                            "url": url,
                        })));
                    });
                }
            }
        });
        nav_watch.replace(Some(handler));
    }
    // Disconnect the watcher once the eval resolves by any path, so it
    // cannot linger on the view and fire for unrelated future loads.
    let unwire = {
        let view = view.clone();
        move || {
            if let Some(id) = nav_watch.borrow_mut().take() {
                view.disconnect(id);
            }
        }
    };

    // Trailing semicolons are meaningless on an expression but would
    // break the `return ( ... )` wrapping; strip them for both the
    // probe and the expression run.
    let trimmed = js.trim().trim_end_matches(';').trim_end().to_string();
    // Compile-only probe: defines an arrow wrapping the expression and
    // returns it without calling it. Parses iff `js` is an expression;
    // never executes user code, so a probe failure carries no side
    // effects and a runtime SyntaxError cannot be mistaken for one.
    let probe = format!("return typeof (async () => (\n{trimmed}\n));");
    let expr = format!("return (\n{trimmed}\n);");
    // Multi-statement scripts without an explicit `return` used to
    // resolve to null, which agents read as "the script broke". Real
    // REPLs answer with the completion value of the last statement;
    // indirect eval gives exactly those semantics, so route return-less
    // bodies through it. Bodies using `return` or `await` keep the
    // plain async-function-body path: `return` because the author
    // already chose the value, `await` because eval code is not an
    // async context and would turn it into a SyntaxError.
    let body = if js.contains("return") || js.contains("await") {
        js
    } else {
        format!(
            "return (0, eval)({});",
            serde_json::to_string(&js).expect("string serializes")
        )
    };
    let view2 = view.clone();
    let cancellable2 = cancellable.clone();
    view.call_async_javascript_function(
        &probe,
        None,
        None,
        None,
        Some(&cancellable),
        move |probed| {
            let source = if probed.is_ok() { expr } else { body };
            view2.call_async_javascript_function(
                &source,
                None,
                None,
                None,
                Some(&cancellable2),
                move |result| {
                    match result {
                        Ok(value) => {
                            unwire();
                            reply.send(Response::value(jsc_to_json(&value)));
                        }
                        Err(e) => {
                            // A cancelled eval means the timeout or the
                            // nav watcher already owns the reply; do not
                            // unwire, a Success-policy load may still be
                            // waiting to resolve it.
                            if !reply.is_spent() && nav_watch_untriggered(&e) {
                                unwire();
                                reply.send(Response::err(format!("eval failed: {e}")));
                            }
                        }
                    }
                },
            );
        },
    );
}

/// True when an eval error is a genuine script failure rather than
/// the cancellation issued by the timeout / navigation watcher.
fn nav_watch_untriggered(e: &glib::Error) -> bool {
    !e.matches(gio::IOErrorEnum::Cancelled)
}

/// Navigate a window; with `wait`, reply once the load reaches
/// `until` (committed / dom / settled).
#[allow(clippy::too_many_arguments)]
pub fn navigate(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
    url: String,
    wait: bool,
    until: LoadStage,
    timeout_ms: Option<u64>,
    reply: Reply,
) {
    let reply = OnceReply::new(reply);
    let win = match resolve(daemon, id) {
        Ok(w) => w,
        Err(resp) => return reply.send(*resp),
    };
    let view = match live_view(&win) {
        Ok(v) => v,
        Err(resp) => return reply.send(*resp),
    };

    let url = crate::ipc_server::normalize_url(url);
    if !wait {
        win.mark_nav_pending(&url);
        view.load_uri(&url);
        return reply.send(Response::window(win.info()));
    }

    arm_timeout(reply.clone(), timeout_ms, "navigate");
    win.mark_nav_pending(&url);
    wire_load_stage(&view, win.clone(), until, {
        let reply = reply.clone();
        move |_| reply.send(Response::window(win.info()))
    });
    view.load_uri(&url);
}

/// Reply once the window's current load reaches `until`.
pub fn wait_load(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
    until: LoadStage,
    timeout_ms: Option<u64>,
    reply: Reply,
) {
    let reply = OnceReply::new(reply);
    let win = match resolve(daemon, id) {
        Ok(w) => w,
        Err(resp) => return reply.send(*resp),
    };
    let view = match live_view(&win) {
        Ok(v) => v,
        Err(resp) => return reply.send(*resp),
    };

    // `is_loading` alone races on a fresh window: `open`/`navigate`
    // issue load_uri, but WebKit only turns that into a live load (and
    // is_loading=true) after a main-loop trip. In that gap wait_load
    // would answer "done" and the caller's next eval gets destroyed by
    // the commit. The window tracks the request (`nav_pending`) until
    // LoadEvent::Started, so cover both.
    let stage_reached = !win.nav_pending()
        && match until {
            LoadStage::Settled => !view.is_loading(),
            LoadStage::Committed | LoadStage::Dom => win.load_committed(),
        };
    if stage_reached {
        if until == LoadStage::Dom {
            // Committed but the DOM may still be parsing; confirm
            // readiness inside the page before releasing the caller.
            arm_timeout(reply.clone(), timeout_ms, "wait_load");
            return await_dom_ready(&view, move |_| reply.send(Response::window(win.info())));
        }
        return reply.send(Response::window(win.info()));
    }
    arm_timeout(reply.clone(), timeout_ms, "wait_load");
    wire_load_stage(&view, win.clone(), until, move |_| {
        reply.send(Response::window(win.info()))
    });
}

/// Detect CAPTCHA / anti-bot challenge UI, and optionally wait for the
/// human/user to clear it. This is intentionally detection-only: it does
/// not call solver services, inject answer tokens, or alter browser
/// fingerprints. The returned JSON is for agent orchestration: pause,
/// present the window if needed, and resume after a manual solve.
pub fn challenge(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
    wait: bool,
    timeout_ms: Option<u64>,
    reply: Reply,
) {
    let ms = timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
    let js = if wait {
        challenge_wait_js(ms)
    } else {
        challenge_detect_js()
    };
    eval_with(daemon, id, js, timeout_ms, NavPolicy::Success, reply);
}

fn challenge_wait_js(timeout_ms: u64) -> String {
    format!(
        r#"{native}{detect}
const started = hwatuNow();
const timeoutMs = {timeout_ms};
let current = detectHwatuChallenge();
while (current.status === 'challenge' && (timeoutMs === 0 || hwatuNow() - started < timeoutMs)) {{
  await hwatuSleep(500);
  current = detectHwatuChallenge();
}}
current.elapsed_ms = hwatuNow() - started;
if (current.status === 'challenge') {{
  current.status = 'manual_required';
  current.manual_required = true;
  current.details = `challenge still present after ${{current.elapsed_ms}} ms; solve it in the browser, then retry or wait again`;
}} else {{
  current.status = 'cleared';
  current.manual_required = false;
  current.details = 'no challenge detected after waiting';
}}
return current;"#,
        native = NATIVE_TIME_JS,
        detect = challenge_detector_js(),
        timeout_ms = timeout_ms,
    )
}

fn challenge_detect_js() -> String {
    format!(
        "{}\nconst result = detectHwatuChallenge();\nresult.elapsed_ms = 0;\nreturn result;",
        CHALLENGE_DETECTOR_JS
    )
}

const CHALLENGE_DETECTOR_JS: &str = r#"
function detectHwatuChallenge() {
  const evidence = [];
  const add = (kind, detail, weight) => evidence.push({ kind, detail: String(detail).slice(0, 160), weight });
  const haystack = `${document.title || ''}\n${document.body ? document.body.innerText : ''}`.toLowerCase();
  const textChecks = [
    ['cloudflare', 'checking if the site connection is secure'],
    ['cloudflare', 'verify you are human'],
    ['generic', 'complete the security check'],
    ['generic', 'prove you are human'],
    ['generic', 'are you a robot'],
  ];
  for (const [kind, phrase] of textChecks) if (haystack.includes(phrase)) add(kind, phrase, 3);

  const nodes = [...document.querySelectorAll('iframe, script, div, input, textarea')];
  for (const el of nodes.slice(0, 400)) {
    const tag = el.tagName.toLowerCase();
    const attrs = ['src', 'title', 'name', 'id', 'class', 'data-sitekey', 'data-testid']
      .map(a => el.getAttribute(a) || '').join(' ').toLowerCase();
    if (!attrs) continue;
    if (attrs.includes('turnstile')) add('turnstile', `${tag} ${attrs}`, 4);
    if (attrs.includes('hcaptcha') || attrs.includes('h-captcha')) add('hcaptcha', `${tag} ${attrs}`, 4);
    if (attrs.includes('recaptcha') || attrs.includes('g-recaptcha')) add('recaptcha', `${tag} ${attrs}`, 4);
    if (attrs.includes('cf-challenge') || attrs.includes('challenge-platform')) add('cloudflare', `${tag} ${attrs}`, 4);
  }

  const weight = evidence.reduce((sum, e) => sum + e.weight, 0);
  const firstStrong = evidence.find(e => e.weight >= 4) || evidence[0];
  const challengeType = firstStrong ? firstStrong.kind : null;
  const confidence = Math.max(0, Math.min(1, weight / 8));
  const present = evidence.length > 0 && confidence >= 0.35;
  return {
    status: present ? 'challenge' : 'clear',
    challenge_type: present ? challengeType : null,
    confidence,
    evidence: evidence.slice(0, 12),
    actionable: present,
    manual_required: present,
    url: location.href,
    title: document.title || '',
  };
}
"#;

pub(crate) fn challenge_detector_js() -> &'static str {
    CHALLENGE_DETECTOR_JS
}

/// Call `done` once on the next `LoadEvent::Finished`, then disconnect.
/// Finished fires for both successful and failed loads (WebKit follows
/// load-failed with Finished), so callers always get an answer.
fn wire_load_finished(view: &webkit6::WebView, done: impl FnOnce(&webkit6::WebView) + 'static) {
    let done = RefCell::new(Some(done));
    let handler: Rc<RefCell<Option<glib::SignalHandlerId>>> = Rc::new(RefCell::new(None));
    let handler2 = handler.clone();
    let id = view.connect_load_changed(move |view, event| {
        if event != webkit6::LoadEvent::Finished {
            return;
        }
        if let Some(id) = handler2.borrow_mut().take() {
            view.disconnect(id);
        }
        if let Some(done) = done.borrow_mut().take() {
            done(view);
        }
    });
    handler.replace(Some(id));
}

/// Like [`wire_load_finished`], but only fires once the window is
/// actually *settled*: no live load and no requested-but-unstarted
/// navigation. A prewarmed view adopted mid-`about:blank` load emits a
/// Finished for that stale load before the real navigation even
/// Starts; counting one Finished there releases the caller into the
/// real page's commit (destroying its evals). Checking quiescence at
/// each Finished instead makes the wait navigation-shaped, not
/// event-shaped.
fn wire_load_settled(
    view: &webkit6::WebView,
    win: Rc<crate::window::BrowserWindow>,
    done: impl FnOnce(&webkit6::WebView) + 'static,
) {
    let done = RefCell::new(Some(done));
    let handler: Rc<RefCell<Option<glib::SignalHandlerId>>> = Rc::new(RefCell::new(None));
    let handler2 = handler.clone();
    let id = view.connect_load_changed(move |view, event| {
        if event != webkit6::LoadEvent::Finished {
            return;
        }
        if win.nav_pending() || view.is_loading() {
            return; // a stale load finished; the requested one is still coming
        }
        if let Some(id) = handler2.borrow_mut().take() {
            view.disconnect(id);
        }
        if let Some(done) = done.borrow_mut().take() {
            done(view);
        }
    });
    handler.replace(Some(id));
}

/// Like [`wire_load_settled`], but fires at the requested load's
/// Committed: the new document has replaced the old one, so evals
/// target the right page even though subresources are still loading.
///
/// Staleness cannot be judged by signal metadata: WebKit updates the
/// view's `uri` property to the requested target as soon as
/// `load_uri` is called, so a stale prewarm/adoption load's Committed
/// already reports the new URL, and its Started can fire after the
/// real navigation was requested. Instead each Committed is verified
/// *inside the committed document*: `location.href` there is ground
/// truth. A stale load is only ever the pool's `about:blank` warmup
/// (see `note_load_engaged`), so any committed document that is not
/// about:blank is the requested one (redirects included); when the
/// target itself is about:blank the two are the same document anyway.
fn wire_load_committed(
    view: &webkit6::WebView,
    win: Rc<crate::window::BrowserWindow>,
    done: impl FnOnce(&webkit6::WebView) + 'static,
) {
    let done = Rc::new(RefCell::new(Some(done)));
    let handler: Rc<RefCell<Option<glib::SignalHandlerId>>> = Rc::new(RefCell::new(None));
    let handler2 = handler.clone();
    let id = view.connect_load_changed(move |view, event| {
        if event != webkit6::LoadEvent::Committed {
            return;
        }
        if win.nav_pending() {
            return; // the requested load has not even Started yet
        }
        if done.borrow().is_none() {
            return; // already released
        }
        let done = done.clone();
        let handler = handler2.clone();
        let view2 = view.clone();
        let target_is_blank = win.nav_target().is_none_or(|t| t.starts_with("about:"));
        view.evaluate_javascript(
            "location.href",
            None,
            None,
            gio::Cancellable::NONE,
            move |result| {
                let href = match &result {
                    Ok(v) => v.to_str().to_string(),
                    // Context destroyed by the next commit: not this
                    // document; a later Committed will verify again.
                    Err(_) => return,
                };
                if href.starts_with("about:blank") && !target_is_blank {
                    return; // the stale warmup document; keep watching
                }
                if let Some(id) = handler.borrow_mut().take() {
                    view2.disconnect(id);
                }
                if let Some(done) = done.borrow_mut().take() {
                    done(&view2);
                }
            },
        );
    });
    handler.replace(Some(id));
}

/// Release `done` once the current document's DOM is fully parsed
/// (`DOMContentLoaded`), checked inside the page so it is exact.
/// Called after Committed, so the document is the requested one.
/// Fires on eval failure too (e.g. a mid-parse redirect destroying
/// the context): the caller's own timeout is the backstop and a
/// stalled wait would be worse than an early release.
fn await_dom_ready(view: &webkit6::WebView, done: impl FnOnce(&webkit6::WebView) + 'static) {
    const JS: &str = r#"if (document.readyState === 'loading') {
  await new Promise((r) => document.addEventListener('DOMContentLoaded', r, { once: true }));
}
return document.readyState;"#;
    let view2 = view.clone();
    view.call_async_javascript_function(JS, None, None, None, gio::Cancellable::NONE, move |_| {
        done(&view2)
    });
}

/// Fire `done` once the window's load reaches `stage`.
fn wire_load_stage(
    view: &webkit6::WebView,
    win: Rc<crate::window::BrowserWindow>,
    stage: LoadStage,
    done: impl FnOnce(&webkit6::WebView) + 'static,
) {
    match stage {
        LoadStage::Settled => wire_load_settled(view, win, done),
        LoadStage::Committed => wire_load_committed(view, win, done),
        LoadStage::Dom => wire_load_committed(view, win, move |view| await_dom_ready(view, done)),
    }
}

/// One-roundtrip verification pass: open a headless window, load
/// `url`, wait for the requested load stage, optionally eval JS and
/// screenshot, close the window (unless `keep`), and reply with
/// everything at once. Collapses the agent inner loop (open,
/// wait-load, eval, shot, close: five process spawns and five socket
/// roundtrips) into a single request.
///
/// Windows are recycled: instead of closing, a finished check parks
/// its window in `daemon.check_pool` and the next check navigates it.
/// A fresh WebKit window pays window construction plus a cold render
/// pipeline (~80 ms to a settled load on a local fixture); navigating
/// a warm window pays ~15 ms. Pooled windows are blanked, their
/// console buffers drained, and closed after [`CHECK_POOL_TTL`] idle.
#[allow(clippy::too_many_arguments)]
pub fn check(
    daemon: &Rc<Daemon>,
    url: Option<String>,
    render: Option<String>,
    base: Option<String>,
    eval_js: Option<String>,
    shot: bool,
    shot_path: Option<String>,
    full: bool,
    baseline: Option<String>,
    tolerance: Option<u8>,
    heatmap: Option<String>,
    until: LoadStage,
    keep: bool,
    timeout_ms: Option<u64>,
    reply: Reply,
) {
    let reply = OnceReply::new(reply);
    let started = std::time::Instant::now();

    // Exactly one input: a URL to navigate to, or markup to render.
    if url.is_some() && render.is_some() {
        return reply.send(Response::err("check takes `url` or `render`, not both"));
    }
    if base.is_some() && render.is_none() {
        return reply.send(Response::err("`base` only applies to `render`"));
    }
    if let Some(html) = &render {
        if html.len() > hwatu_ipc::RENDER_MAX_BYTES {
            return reply.send(Response::err(format!(
                "render document is {} bytes; the cap is {} (write it to a file \
                 and serve it over http instead)",
                html.len(),
                hwatu_ipc::RENDER_MAX_BYTES
            )));
        }
    }

    let rendered = render.is_some();
    let (win, view, adopted) = if let Some(html) = render {
        let base = base.map(crate::ipc_server::normalize_url);
        match acquire_render_window(daemon, &html, base.as_deref()) {
            Ok((win, view)) => (win, view, false),
            Err(resp) => return reply.send(*resp),
        }
    } else {
        let Some(url) = url else {
            return reply.send(Response::err("check needs `url` or `render`"));
        };
        let url = crate::ipc_server::normalize_url(url);
        // A prefetched window for this URL is already loading (or
        // loaded): adopt it and skip the navigation entirely.
        let prefetched = claim_prefetch(daemon, &url);
        let adopted = prefetched.is_some();
        match prefetched
            .map(Ok)
            .unwrap_or_else(|| acquire_check_window(daemon, &url))
        {
            Ok((win, view)) => (win, view, adopted),
            Err(resp) => return reply.send(*resp),
        }
    };
    let win_id = win.id;
    daemon.last_target.replace(Some(win_id));

    // One deadline covers the whole pass. The timeout also closes the
    // window (unless `keep`): a check that timed out must not leak a
    // headless window the agent never learns the id of.
    {
        let reply = reply.clone();
        let daemon = daemon.clone();
        let ms = timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
        if ms > 0 {
            glib::timeout_add_local_once(std::time::Duration::from_millis(ms), move || {
                if !reply.is_spent() {
                    close_check_window(&daemon, win_id, keep);
                    reply.send(Response::err(format!("check timed out after {ms} ms")));
                }
            });
        }
    }

    let daemon = daemon.clone();
    let proceed = move |view: &webkit6::WebView| {
        if reply.is_spent() {
            return; // timeout already answered (and closed the window)
        }
        let load_ms = started.elapsed().as_millis() as u64;
        let result = Rc::new(RefCell::new(serde_json::json!({
            "url": view.uri().map(|u| u.to_string()).unwrap_or_default(),
            "load_ms": load_ms,
        })));
        if keep {
            result.borrow_mut()["id"] = serde_json::json!(win_id);
        }
        if adopted {
            // The load was already warm from a `prefetch`; load_ms
            // measures adoption, not a navigation.
            result.borrow_mut()["prefetched"] = serde_json::json!(true);
        }
        if rendered {
            result.borrow_mut()["rendered"] = serde_json::json!(true);
        }

        // Eval and screenshot are independent of each other; run both
        // concurrently and reply when the last one lands.
        let pending = Rc::new(Cell::new(1u32));
        let finish = {
            let daemon = daemon.clone();
            let result = result.clone();
            let reply = reply.clone();
            let pending = pending.clone();
            let view = view.clone();
            Rc::new(move || {
                pending.set(pending.get() - 1);
                if pending.get() > 0 || reply.is_spent() {
                    return;
                }
                // Console capture last: it sees everything the load
                // and the eval produced. Drained (`clear`) because the
                // window may be recycled for the next check.
                let console = daemon
                    .windows
                    .borrow()
                    .get(&win_id)
                    .map(|w| w.console.read(true, Some(20)))
                    .unwrap_or_default();
                {
                    let mut r = result.borrow_mut();
                    if !console.is_empty() {
                        r["console"] = serde_json::to_value(&console).unwrap_or_default();
                    }
                }
                // Title from the DOM, not view.title(): WebKit
                // publishes the title property asynchronously after
                // the load, so on fast/recycled checks it lags (or
                // holds the previous page's value).
                let daemon = daemon.clone();
                let result = result.clone();
                let reply = reply.clone();
                let started = started;
                view.evaluate_javascript(
                    "document.title",
                    None,
                    None,
                    gio::Cancellable::NONE,
                    move |title| {
                        {
                            let mut r = result.borrow_mut();
                            r["title"] = serde_json::json!(title
                                .ok()
                                .map(|v| v.to_str().to_string())
                                .unwrap_or_default());
                            r["total_ms"] = serde_json::json!(started.elapsed().as_millis() as u64);
                        }
                        release_check_window(&daemon, win_id, keep);
                        reply.send(Response::value(result.borrow().clone()));
                    },
                );
            })
        };

        if let Some(js) = eval_js {
            pending.set(pending.get() + 1);
            let finish = finish.clone();
            let result = result.clone();
            eval(
                &daemon,
                Some(win_id),
                js,
                timeout_ms,
                Box::new(move |resp| {
                    result.borrow_mut()["eval"] = match resp {
                        Response::Ok { value, .. } => value.unwrap_or(serde_json::Value::Null),
                        Response::Err { message } => serde_json::json!({ "error": message }),
                    };
                    finish();
                }),
            );
        }
        if shot || shot_path.is_some() {
            pending.set(pending.get() + 1);
            let finish = finish.clone();
            let result = result.clone();
            screenshot(
                &daemon,
                Some(win_id),
                shot_path,
                full,
                Box::new(move |resp| {
                    match resp {
                        Response::Ok { path: Some(p), .. } => {
                            result.borrow_mut()["shot"] = serde_json::json!(p);
                        }
                        Response::Err { message } => {
                            result.borrow_mut()["shot"] = serde_json::json!({ "error": message });
                        }
                        _ => {}
                    }
                    finish();
                }),
            );
        }
        if let Some(png) = baseline {
            pending.set(pending.get() + 1);
            let finish = finish.clone();
            let result = result.clone();
            crate::verify::diff_against_baseline(
                &daemon,
                win_id,
                png,
                tolerance,
                heatmap,
                full,
                Box::new(move |value| {
                    result.borrow_mut()["diff"] = match value {
                        Ok(v) => v,
                        Err(e) => serde_json::json!({ "error": e }),
                    };
                    finish();
                }),
            );
        }
        finish(); // release the base hold; replies if nothing else is pending
    };

    // An adopted prefetch window's load may have engaged before this
    // request arrived, so the requested stage's signal may already
    // have fired; waiting for it again would stall until timeout.
    // Once the document has Committed (and no navigation is pending),
    // dispatch on what is left: nothing loading means every stage
    // passed, otherwise committed/dom resolve in-page and settled
    // waits for the still-coming Finished. A prefetch that has not
    // Committed yet behaves exactly like a fresh check navigation
    // (prefetch marks nav_pending the same way), so the normal stage
    // wiring is correct for it.
    if adopted && win.load_committed() && !win.nav_pending() {
        if !view.is_loading() {
            let view = view.clone();
            glib::idle_add_local_once(move || proceed(&view));
        } else {
            match until {
                LoadStage::Committed => {
                    let view = view.clone();
                    glib::idle_add_local_once(move || proceed(&view));
                }
                LoadStage::Dom => await_dom_ready(&view, proceed),
                LoadStage::Settled => wire_load_settled(&view, win.clone(), proceed),
            }
        }
    } else {
        wire_load_stage(&view, win.clone(), until, proceed);
    }
}

/// How long an idle pooled check window is kept before being closed.
const CHECK_POOL_TTL: std::time::Duration = std::time::Duration::from_secs(60);

/// Cap on parked check windows; beyond this, released windows close.
const CHECK_POOL_MAX: usize = 2;

/// Get a window for a check: reuse a parked one (navigate it to
/// `url`) or open a fresh headless window. Returns the window with
/// the navigation already requested, exactly like `BrowserWindow::open`.
fn acquire_check_window(
    daemon: &Rc<Daemon>,
    url: &str,
) -> Result<(Rc<BrowserWindow>, webkit6::WebView), Box<Response>> {
    let (win, view) = acquire_pooled_window(daemon, url.starts_with("file:"))?;
    win.mark_nav_pending(url);
    view.load_uri(url);
    Ok((win, view))
}

/// Like [`acquire_check_window`], but loads inline markup directly
/// (`load_html`) instead of navigating. The document's URI becomes
/// `base` (which also resolves the markup's relative references), or
/// a unique `file:///hwatu-render/<n>/` when the caller gave none.
///
/// The fallback is deliberately NOT about:blank: the stale-load
/// machinery (`note_load_engaged`, the committed-document check)
/// distinguishes a requested load from a pool window's leftover
/// blanking load by "stale loads are only ever about:blank", and a
/// render targeting about:blank on a still-blanking recycled window
/// would be indistinguishable from its own stale load and could
/// release waits on the wrong document. And it is deliberately a
/// `file:` URI: measured on this box, load_html against custom
/// (`hwatu://`, unregistered) or unresolvable-http bases stalls the
/// commit ~500-700 ms (scheme/DNS resolution in the network process),
/// while `file:`/`about:` bases commit in single-digit ms. The path
/// does not exist, so the markup's relative references resolve to
/// nothing rather than to real files; reading local files via
/// fetch/XHR stays blocked by WebKit's default file-URL policy.
fn acquire_render_window(
    daemon: &Rc<Daemon>,
    html: &str,
    base: Option<&str>,
) -> Result<(Rc<BrowserWindow>, webkit6::WebView), Box<Response>> {
    let base = match base {
        Some(b) => b.to_string(),
        None => format!("file:///hwatu-render/{}/", daemon.alloc_id()),
    };
    let (win, view) = acquire_pooled_window(daemon, base.starts_with("file:"))?;
    win.mark_nav_pending(&base);
    view.load_html(html, Some(&base));
    Ok((win, view))
}

/// Pop a live parked check window whose last document's origin kind
/// matches the incoming load (`want_file`), or open a fresh headless
/// one. The caller starts its own load (URL or inline markup) on the
/// returned view.
///
/// The taint match matters: WebKit swaps the web process when a
/// navigation crosses the file:/network boundary, and the swap costs
/// more than a fresh window (~650 ms vs ~240 ms measured here). A
/// mismatched park is left parked (its TTL still applies) rather
/// than adopted or evicted, so alternating render/check loops keep
/// one warm window per origin kind.
fn acquire_pooled_window(
    daemon: &Rc<Daemon>,
    want_file: bool,
) -> Result<(Rc<BrowserWindow>, webkit6::WebView), Box<Response>> {
    loop {
        let entry = {
            let mut pool = daemon.check_pool.borrow_mut();
            match pool.iter().rposition(|&(_, _, file)| file == want_file) {
                Some(i) => pool.remove(i),
                None => break,
            }
        };
        let (id, _token, _file) = entry;
        let Some(win) = daemon.windows.borrow().get(&id).cloned() else {
            continue; // closed behind our back (e.g. `hwatu close`)
        };
        let Ok(view) = live_view(&win) else {
            close_check_window(daemon, id, false);
            continue;
        };
        return Ok((win, view));
    }
    // Fresh windows open at about:blank, never the launcher: the
    // caller's load replaces it immediately, and the stale-blank
    // machinery (`note_load_engaged`, the stage wiring) already knows
    // how to ignore a superseded about:blank load. A launcher load
    // here would clear `nav_pending` and release waits early.
    let info = BrowserWindow::open(
        daemon,
        Some("about:blank".to_string()),
        None,
        OpenMode::Headless,
    );
    let win = daemon
        .windows
        .borrow()
        .get(&info.id)
        .cloned()
        .ok_or_else(|| Box::new(Response::err("check: window vanished at open")))?;
    let view = live_view(&win).inspect_err(|_| {
        close_check_window(daemon, info.id, false);
    })?;
    Ok((win, view))
}

/// Return a check's window: park it for reuse (blanked, console
/// drained) or close it when the pool is full / the caller kept it.
/// A TTL timer closes parked windows that nobody reuses, so a burst
/// of checks does not permanently raise the daemon's memory floor.
fn release_check_window(daemon: &Rc<Daemon>, id: u64, keep: bool) {
    if keep {
        return;
    }
    let Some(win) = daemon.windows.borrow().get(&id).cloned() else {
        return;
    };
    if daemon.check_pool.borrow().len() >= CHECK_POOL_MAX {
        close_check_window(daemon, id, false);
        return;
    }
    // Blank the page so the parked window drops the page's memory and
    // can't keep running its scripts/timers between checks. The
    // file-origin taint is judged BEFORE blanking: about:blank stays
    // in the current web process, so a window that just held a file:
    // document (a render) keeps its file-capable process across the
    // blank, and only same-kind loads may adopt it cheaply.
    win.console.read(true, None); // drain
    let was_file = win
        .live_webview()
        .and_then(|v| v.uri())
        .map(|u| u.starts_with("file:"))
        .unwrap_or(false);
    if let Some(view) = win.live_webview() {
        win.mark_nav_pending("about:blank");
        view.load_uri("about:blank");
    }
    // Unique token: a TTL timer only closes the park it was armed for.
    let token = {
        let mut n = daemon.next_id.borrow_mut();
        let t = *n;
        *n += 1;
        t
    };
    daemon.check_pool.borrow_mut().push((id, token, was_file));
    let daemon = daemon.clone();
    glib::timeout_add_local_once(CHECK_POOL_TTL, move || {
        // Only close if still parked from THIS park (not reacquired).
        let parked = {
            let mut pool = daemon.check_pool.borrow_mut();
            match pool.iter().position(|&(i, t, _)| (i, t) == (id, token)) {
                Some(i) => {
                    pool.remove(i);
                    true
                }
                None => false,
            }
        };
        if parked {
            close_check_window(&daemon, id, false);
        }
    });
}

/// Close a check's window, unless the caller asked to keep it.
fn close_check_window(daemon: &Rc<Daemon>, id: u64, keep: bool) {
    if keep {
        return;
    }
    // Bind before closing: `close()` re-enters the window's
    // close-request handler, which borrows `daemon.windows` itself, so
    // the borrow_mut must be released first (if-let would hold it).
    let win = daemon.windows.borrow_mut().remove(&id);
    if let Some(win) = win {
        win.close();
    }
}

/// How long an unclaimed prefetch is kept before its window returns
/// to the check pool. Short on purpose: a prefetched page goes stale
/// the moment the dev server rebuilds, and a check adopting stale
/// content would verify the wrong build.
const PREFETCH_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// Cap on outstanding prefetch windows: speculation must never raise
/// the daemon's memory floor unboundedly.
const PREFETCH_MAX: usize = 3;

/// Speculatively load `url` in a headless window and reply
/// immediately. The next `check` of the same URL adopts the window
/// (see [`claim_prefetch`]); an unclaimed prefetch is released to the
/// check pool after [`PREFETCH_TTL`]. Prefetching a URL that already
/// has an outstanding prefetch re-navigates that window (the page may
/// have been rebuilt since), rather than stacking a second one.
pub fn prefetch(daemon: &Rc<Daemon>, url: String, reply: Reply) {
    let url = crate::ipc_server::normalize_url(url);

    // Re-navigate an existing prefetch of the same URL in place.
    let existing = {
        let pool = daemon.prefetch_pool.borrow();
        pool.iter().find(|(u, ..)| *u == url).map(|&(_, id, _)| id)
    };
    if let Some(id) = existing {
        if let Some(view) = daemon.windows.borrow().get(&id).and_then(|w| {
            w.mark_nav_pending(&url);
            w.live_webview()
        }) {
            view.load_uri(&url);
            return reply(Response::value(
                serde_json::json!({ "prefetching": url, "id": id }),
            ));
        }
        // Window vanished behind our back: drop the stale entry.
        daemon
            .prefetch_pool
            .borrow_mut()
            .retain(|(_, i, _)| *i != id);
    }

    if daemon.prefetch_pool.borrow().len() >= PREFETCH_MAX {
        // Evict the oldest speculation: newer intent is better intent.
        let (_, old_id, _) = daemon.prefetch_pool.borrow_mut().remove(0);
        release_check_window(daemon, old_id, false);
    }

    let (win, _view) = match acquire_check_window(daemon, &url) {
        Ok(pair) => pair,
        Err(resp) => return reply(*resp),
    };
    let win_id = win.id;
    let token = daemon.alloc_id();
    daemon
        .prefetch_pool
        .borrow_mut()
        .push((url.clone(), win_id, token));

    // TTL: an unclaimed prefetch returns to the ordinary check pool
    // (blanked), so speculation misses cost only the load, not RAM.
    {
        let daemon = daemon.clone();
        glib::timeout_add_local_once(PREFETCH_TTL, move || {
            let expired = {
                let mut pool = daemon.prefetch_pool.borrow_mut();
                match pool.iter().position(|&(_, i, t)| (i, t) == (win_id, token)) {
                    Some(idx) => {
                        pool.remove(idx);
                        true
                    }
                    None => false, // claimed, or re-navigated (new token)
                }
            };
            if expired {
                release_check_window(&daemon, win_id, false);
            }
        });
    }

    reply(Response::value(
        serde_json::json!({ "prefetching": url, "id": win_id }),
    ))
}

/// Claim the prefetched window for `url`, if one is outstanding and
/// still alive. The caller (check) adopts its in-flight load.
fn claim_prefetch(daemon: &Rc<Daemon>, url: &str) -> Option<(Rc<BrowserWindow>, webkit6::WebView)> {
    let (id, _token) = {
        let mut pool = daemon.prefetch_pool.borrow_mut();
        let idx = pool.iter().position(|(u, ..)| u == url)?;
        let (_, id, token) = pool.remove(idx);
        (id, token)
    };
    let win = daemon.windows.borrow().get(&id).cloned()?;
    let Some(view) = win.live_webview() else {
        close_check_window(daemon, id, false); // discarded: unusable
        return None;
    };
    Some((win, view))
}

/// Capture the page as a PNG on disk: the visible viewport, or the
/// entire document with `full` (WebKit renders the whole scrollable
/// area, no scroll-and-stitch needed).
pub fn screenshot(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
    path: Option<String>,
    full: bool,
    reply: Reply,
) {
    let reply = OnceReply::new(reply);
    let win = match resolve(daemon, id) {
        Ok(w) => w,
        Err(resp) => return reply.send(*resp),
    };
    let view = match live_view(&win) {
        Ok(v) => v,
        Err(resp) => return reply.send(*resp),
    };

    let target = path.map(std::path::PathBuf::from).unwrap_or_else(|| {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("hwatu-shot-{}-{ts}.png", win.id))
    });

    let region = if full {
        webkit6::SnapshotRegion::FullDocument
    } else {
        webkit6::SnapshotRegion::Visible
    };
    view.snapshot(
        region,
        webkit6::SnapshotOptions::NONE,
        gio::Cancellable::NONE,
        move |result| match result {
            Ok(texture) => {
                // Download the pixels here (a memcpy), then encode and
                // write on a worker thread: PNG compression of a
                // 2048x1536 frame costs tens of ms and used to stall
                // the GTK main loop (and with it every other window)
                // inside cairo's zlib-best path. The `png` crate with
                // fast filtering is also ~2x quicker.
                let mut downloader = gdk::TextureDownloader::new(&texture);
                downloader.set_format(gdk::MemoryFormat::R8g8b8a8);
                let (bytes, stride) = downloader.download_bytes();
                let width = texture.width() as u32;
                let height = texture.height() as u32;
                glib::spawn_future_local(async move {
                    let path = target.clone();
                    let encoded = gio::spawn_blocking(move || {
                        write_png(&path, &bytes, stride, width, height)
                    })
                    .await;
                    match encoded {
                        Ok(Ok(())) => reply.send(Response::path(target.to_string_lossy())),
                        Ok(Err(e)) => reply.send(Response::err(format!(
                            "screenshot write to {} failed: {e}",
                            target.display()
                        ))),
                        Err(_) => reply.send(Response::err("screenshot encode panicked")),
                    }
                });
            }
            Err(e) => reply.send(Response::err(format!("screenshot failed: {e}"))),
        },
    );
}

/// Encode RGBA rows (possibly padded to `stride`) as a PNG. Fast
/// compression + Sub filtering: screenshots are verification
/// artifacts, so encode speed beats squeezing out the last few KB.
fn write_png(
    path: &std::path::Path,
    rgba: &[u8],
    stride: usize,
    width: u32,
    height: u32,
) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    encoder.set_filter(png::FilterType::Sub);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    let row = width as usize * 4;
    if stride == row {
        writer.write_image_data(rgba).map_err(|e| e.to_string())?;
    } else {
        let mut tight = Vec::with_capacity(row * height as usize);
        for y in 0..height as usize {
            tight.extend_from_slice(&rgba[y * stride..y * stride + row]);
        }
        writer.write_image_data(&tight).map_err(|e| e.to_string())?;
    }
    writer.finish().map_err(|e| e.to_string())
}

/// Set a file input's files from a path on disk. The bytes travel
/// into the page as base64 inside a JS snippet, are decoded to a
/// `File`, and assigned through `DataTransfer`, the same technique
/// every automation harness uses, since engines allow assigning
/// `input.files` but not opening the OS picker programmatically.
pub fn upload(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
    selector: String,
    path: String,
    timeout_ms: Option<u64>,
    reply: Reply,
) {
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            return OnceReply::new(reply).send(Response::err(format!("cannot read {path}: {e}")));
        }
    };
    // Keep IPC and JS-source sizes sane: 32 MiB of payload is ~43 MiB
    // of base64 in a script string. Enough for fixtures and documents;
    // bulk uploads should not go through a JS harness anyway.
    const MAX: usize = 32 * 1024 * 1024;
    if bytes.len() > MAX {
        return OnceReply::new(reply).send(Response::err(format!(
            "{path} is {} bytes; upload supports at most {MAX} (32 MiB)",
            bytes.len()
        )));
    }
    let name = std::path::Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "upload.bin".into());
    let mime = mime_from_extension(&path);
    let js = format!(
        r#"const selector = {selector};
const el = document.querySelector(selector);
if (!el) throw new Error('no element matches selector: ' + selector);
if (!(el instanceof HTMLInputElement) || el.type !== 'file')
  throw new Error('element is not an <input type=file>: ' + selector);
const bin = atob({b64});
const bytes = new Uint8Array(bin.length);
for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
const file = new File([bytes], {name}, {{ type: {mime} }});
const dt = new DataTransfer();
dt.items.add(file);
el.files = dt.files;
el.dispatchEvent(new Event('input', {{ bubbles: true }}));
el.dispatchEvent(new Event('change', {{ bubbles: true }}));
return {{ uploaded: file.name, size: file.size, type: file.type }};"#,
        selector = js_string(&selector),
        b64 = js_string(&base64(&bytes)),
        name = js_string(&name),
        mime = js_string(mime),
    );
    eval(daemon, id, js, timeout_ms, reply);
}

/// Scroll the page and report where it landed. One of `selector`
/// (scrolled into view, centered, disambiguated by `nth`/`contains`),
/// `to_y` (absolute pixels), or `by_pages` (relative viewport-heights,
/// default 1.0). Implemented as an eval so the response can carry
/// what actually happened: match count, the matched element's tag and
/// text, and the final scroll position, so an agent never has to
/// screenshot just to learn whether a scroll hit the right thing.
#[allow(clippy::too_many_arguments)]
pub fn scroll(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
    selector: Option<String>,
    nth: Option<u32>,
    contains: Option<String>,
    to_y: Option<f64>,
    by_pages: Option<f64>,
    timeout_ms: Option<u64>,
    reply: Reply,
) {
    let modes = [selector.is_some(), to_y.is_some(), by_pages.is_some()];
    if modes.iter().filter(|m| **m).count() > 1 {
        return OnceReply::new(reply).send(Response::err(
            "scroll takes exactly one of selector, to_y, by_pages",
        ));
    }
    let js = format!(
        r#"{native}const selector = {selector};
const nth = {nth};
const contains = {contains};
const toY = {to_y};
const byPages = {by_pages};
if (selector !== null) {{
  let els = [...document.querySelectorAll(selector)];
  const total = els.length;
  if (contains !== null)
    els = els.filter(e => ((e.textContent || '') + ' ' + (e.value || '')).includes(contains));
  const el = els[nth];
  if (!el) {{
    const filt = contains === null ? '' : ` (${{els.length}} after contains filter)`;
    throw new Error(`no match: ${{total}} element(s) for selector${{filt}}, nth=${{nth}}`);
  }}
  el.scrollIntoView({{ block: 'center', behavior: 'instant' }});
  var matched = {{
    matches: els.length,
    tag: el.tagName.toLowerCase(),
    text: (el.textContent || '').trim().slice(0, 120),
  }};
}} else if (toY !== null) {{
  window.scrollTo({{ top: toY, behavior: 'instant' }});
}} else {{
  window.scrollBy({{ top: window.innerHeight * (byPages ?? 1.0), behavior: 'instant' }});
}}
// Let the instant scroll settle before measuring. Not rAF: frames
// never fire in headless/unmapped windows, which are exactly where
// agent scrolls run. scrollTo/scrollBy with behavior:'instant'
// update scrollY synchronously; one macrotask covers layout shifts.
// Native-clock sleep so a paused virtual clock cannot stall it.
await hwatuSleep(0);
const doc = document.documentElement;
const maxY = Math.max(0, doc.scrollHeight - window.innerHeight);
return {{
  x: window.scrollX,
  y: window.scrollY,
  max_y: maxY,
  at_bottom: window.scrollY >= maxY - 1,
  ...(typeof matched === 'undefined' ? {{}} : {{ matched }}),
}};"#,
        native = NATIVE_TIME_JS,
        selector = json_or_null(selector.as_deref()),
        nth = nth.unwrap_or(0),
        contains = json_or_null(contains.as_deref()),
        to_y = to_y.map_or("null".into(), |v| v.to_string()),
        by_pages = by_pages.map_or("null".into(), |v| v.to_string()),
    );
    eval(daemon, id, js, timeout_ms, reply);
}

/// JS literal for an optional string: a JSON string or `null`.
fn json_or_null(s: Option<&str>) -> String {
    s.map_or("null".into(), js_string)
}

/// Assert page state, polling inside the page until it holds or the
/// deadline passes. Success replies `{ok: true, matches, tag, text,
/// elapsed_ms}`; failure is a structured error naming what WAS found
/// (match count, the element's actual text) so the agent can act on
/// it without a follow-up snapshot.
#[allow(clippy::too_many_arguments)]
pub fn expect(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
    selector: String,
    nth: Option<u32>,
    contains: Option<String>,
    text: Option<String>,
    absent: bool,
    visible: bool,
    timeout_ms: Option<u64>,
    reply: Reply,
) {
    let deadline = timeout_ms.unwrap_or(5000);
    let js = format!(
        r#"{native}const selector = {selector};
const nth = {nth};
const contains = {contains};
const wantText = {want_text};
const absent = {absent};
const wantVisible = {visible};
const deadline = {deadline};
const started = hwatuNow();

// Visibility that a bare existence check misses: zero-sized boxes,
// display/visibility/opacity hiding (on the element or an ancestor,
// getComputedStyle inherits the first two, opacity needs the walk),
// and occlusion, where the element renders but another element is on
// top (wrong z-index, forgotten overlay). elementFromPoint at the
// center answers occlusion directly; a hit inside the subtree (or a
// label/shadow host relationship) counts as visible.
function whyInvisible(el) {{
  for (let n = el; n instanceof Element; n = n.parentElement) {{
    const st = getComputedStyle(n);
    if (st.display === 'none') return n === el ? 'display:none' : 'display:none on ancestor <' + n.tagName.toLowerCase() + '>';
    if (st.visibility === 'hidden' || st.visibility === 'collapse') return 'visibility:' + st.visibility + (n === el ? '' : ' on ancestor <' + n.tagName.toLowerCase() + '>');
    if (st.opacity === '0') return n === el ? 'opacity:0' : 'opacity:0 on ancestor <' + n.tagName.toLowerCase() + '>';
  }}
  const r = el.getBoundingClientRect();
  if (r.width === 0 || r.height === 0) return `zero-size box (${{r.width}}x${{r.height}})`;
  const cx = Math.min(Math.max(r.left + r.width / 2, 0), innerWidth - 1);
  const cy = Math.min(Math.max(r.top + r.height / 2, 0), innerHeight - 1);
  if (r.bottom < 0 || r.top > innerHeight || r.right < 0 || r.left > innerWidth)
    return `outside viewport (rect ${{Math.round(r.left)}},${{Math.round(r.top)}} ${{Math.round(r.width)}}x${{Math.round(r.height)}})`;
  const hit = document.elementFromPoint(cx, cy);
  if (hit && (hit === el || el.contains(hit) || hit.contains(el)
      || (hit.shadowRoot && hit.shadowRoot.contains(el))
      || (hit instanceof HTMLLabelElement && hit.control === el)))
    return null;
  if (!hit) return 'center point hits nothing (elementFromPoint returned null)';
  const t = (hit.textContent || '').trim().slice(0, 60);
  return `covered by <${{hit.tagName.toLowerCase()}}${{hit.id ? '#' + hit.id : ''}}> ${{JSON.stringify(t)}}`;
}}

function check() {{
  let els = [...document.querySelectorAll(selector)];
  const total = els.length;
  if (contains !== null)
    els = els.filter(e => ((e.textContent || '') + ' ' + (e.value || '')).includes(contains));
  const el = els[nth];
  if (absent) {{
    if (els.length === 0) return {{ ok: true }};
    const t = (els[0].textContent || '').trim().slice(0, 120);
    return {{ ok: false, why: `expected no match, found ${{els.length}} (first: <${{els[0].tagName.toLowerCase()}}> ${{JSON.stringify(t)}})` }};
  }}
  if (!el) {{
    const filt = contains === null ? '' : ` (${{els.length}} after contains filter)`;
    return {{ ok: false, why: `no match: ${{total}} element(s) for selector${{filt}}, nth=${{nth}}` }};
  }}
  const actual = (el.textContent || '').trim();
  if (wantText !== null && !actual.includes(wantText)) {{
    return {{ ok: false, why: `text mismatch: expected to contain ${{JSON.stringify(wantText)}}, got ${{JSON.stringify(actual.slice(0, 200))}}` }};
  }}
  if (wantVisible) {{
    const why = whyInvisible(el);
    if (why) return {{ ok: false, why: `element matched but is not visible: ${{why}}` }};
  }}
  return {{
    ok: true,
    matches: els.length,
    tag: el.tagName.toLowerCase(),
    text: actual.slice(0, 120),
  }};
}}

let result = check();
while (!result.ok && hwatuNow() - started < deadline) {{
  await hwatuSleep(100);
  result = check();
}}
result.elapsed_ms = hwatuNow() - started;
if (!result.ok) throw new Error(`expect failed after ${{result.elapsed_ms}} ms: ${{result.why}}`);
return result;"#,
        native = NATIVE_TIME_JS,
        selector = js_string(&selector),
        nth = nth.unwrap_or(0),
        contains = json_or_null(contains.as_deref()),
        want_text = json_or_null(text.as_deref()),
        absent = absent,
        visible = visible,
        deadline = deadline,
    );
    // The page-side loop owns the deadline; give the eval transport a
    // margin above it so the structured failure (not a generic eval
    // timeout) is what the caller sees.
    eval(daemon, id, js, Some(deadline + 2000), reply);
}

/// Token-cheap page state: url, title, bounded visible text, and an
/// indexed list of interactable elements (links, buttons, inputs...).
/// The elements are remembered on `window.__hwatu_refs`, so a
/// follow-up click/type can target `ref: n` without a selector. The
/// designed alternative to screenshot-and-squint for agents.
pub fn snapshot(daemon: &Rc<Daemon>, id: Option<u64>, timeout_ms: Option<u64>, reply: Reply) {
    const JS: &str = r#"
const MAX_TEXT = 4000;
const MAX_ELS = 120;
const clip = (s, n) => {
  s = (s || '').replace(/\s+/g, ' ').trim();
  return s.length > n ? s.slice(0, n - 1) + '…' : s;
};
const visible = (el) => {
  const r = el.getBoundingClientRect();
  if (r.width === 0 && r.height === 0) return false;
  const st = getComputedStyle(el);
  return st.visibility !== 'hidden' && st.display !== 'none';
};
const label = (el) => {
  const aria = el.getAttribute('aria-label');
  if (aria) return aria;
  if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement) {
    if (el.labels && el.labels.length) return el.labels[0].textContent;
    return el.placeholder || el.name || el.value || '';
  }
  if (el instanceof HTMLImageElement) return el.alt;
  return el.textContent;
};
const sel = 'a[href], button, input, textarea, select, [role=button], [role=link], [role=tab], [role=menuitem], [role=checkbox], [contenteditable=""], [contenteditable=true], [onclick]';
const els = [...document.querySelectorAll(sel)].filter(visible).slice(0, MAX_ELS);
window.__hwatu_refs = els;
const interactables = els.map((el, i) => {
  const out = { ref: i, tag: el.tagName.toLowerCase() };
  const text = clip(label(el), 80);
  if (text) out.text = text;
  if (el.id) out.id = el.id;
  if (el instanceof HTMLInputElement) {
    out.type = el.type;
    if (el.value && el.type !== 'password') out.value = clip(el.value, 40);
    if (el.checked) out.checked = true;
  }
  if (el.name) out.name = el.name;
  if (el instanceof HTMLAnchorElement && el.href) out.href = clip(el.href, 120);
  if (el.disabled) out.disabled = true;
  return out;
});
const doc = document.documentElement;
return {
  url: location.href,
  title: document.title,
  text: clip(document.body ? document.body.innerText : '', MAX_TEXT),
  interactables,
  scroll: {
    y: window.scrollY,
    max_y: Math.max(0, doc.scrollHeight - window.innerHeight),
  },
};"#;
    eval(daemon, id, JS.to_string(), timeout_ms, reply);
}

/// JS prelude that resolves a click/type target: by snapshot `ref` or
/// by selector + nth/contains (same disambiguation as scroll). Leaves
/// `el` (the element) and `matched` (the landing report) in scope, or
/// throws with an explanation an agent can act on.
fn target_prelude(
    selector: Option<&str>,
    nth: Option<u32>,
    contains: Option<&str>,
    ref_idx: Option<u32>,
) -> Result<String, Box<Response>> {
    if selector.is_some() == ref_idx.is_some() {
        return Err(Box::new(Response::err(
            "pass exactly one of a CSS selector or --ref <n> (from `hwatu snapshot`)",
        )));
    }
    Ok(format!(
        r#"const selector = {selector};
const nth = {nth};
const contains = {contains};
const refIdx = {ref_idx};
let el;
if (refIdx !== null) {{
  const refs = window.__hwatu_refs;
  if (!refs) throw new Error('no snapshot taken; run `hwatu snapshot` first or use a selector');
  el = refs[refIdx];
  if (!el) throw new Error(`ref ${{refIdx}} out of range (snapshot had ${{refs.length}} interactables)`);
  if (!el.isConnected) throw new Error(`ref ${{refIdx}} is no longer in the document; re-run snapshot`);
  var matched = {{ ref: refIdx }};
}} else {{
  let els = [...document.querySelectorAll(selector)];
  const total = els.length;
  if (contains !== null)
    els = els.filter(e => ((e.textContent || '') + ' ' + (e.value || '')).includes(contains));
  el = els[nth];
  if (!el) {{
    const filt = contains === null ? '' : ` (${{els.length}} after contains filter)`;
    throw new Error(`no match: ${{total}} element(s) for selector${{filt}}, nth=${{nth}}`);
  }}
  var matched = {{ matches: els.length }};
}}
matched.tag = el.tagName.toLowerCase();
matched.text = (el.textContent || el.value || '').trim().slice(0, 120);
"#,
        selector = json_or_null(selector),
        nth = nth.unwrap_or(0),
        contains = json_or_null(contains),
        ref_idx = ref_idx.map_or("null".into(), |v| v.to_string()),
    ))
}

/// Click an element with a real pointer-event sequence at the
/// element's center (pages listening on pointerdown/mousedown see the
/// same shape as a human click). Reports what was hit.
#[allow(clippy::too_many_arguments)]
pub fn click(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
    selector: Option<String>,
    nth: Option<u32>,
    contains: Option<String>,
    ref_idx: Option<u32>,
    timeout_ms: Option<u64>,
    reply: Reply,
) {
    let prelude = match target_prelude(selector.as_deref(), nth, contains.as_deref(), ref_idx) {
        Ok(p) => p,
        Err(resp) => return OnceReply::new(reply).send(*resp),
    };
    let js = format!(
        r#"{native}{prelude}
el.scrollIntoView({{ block: 'center', behavior: 'instant' }});
const r = el.getBoundingClientRect();
const x = r.left + r.width / 2, y = r.top + r.height / 2;
const opts = {{ bubbles: true, cancelable: true, composed: true, view: window,
                clientX: x, clientY: y, button: 0, detail: 1 }};
el.dispatchEvent(new PointerEvent('pointerdown', {{ ...opts, pointerId: 1, isPrimary: true }}));
el.dispatchEvent(new MouseEvent('mousedown', opts));
if (el.focus) el.focus();
el.dispatchEvent(new PointerEvent('pointerup', {{ ...opts, pointerId: 1, isPrimary: true }}));
el.dispatchEvent(new MouseEvent('mouseup', opts));
el.click ? el.click() : el.dispatchEvent(new MouseEvent('click', opts));
// Give a same-document reaction (SPA routing, form handlers) one
// macrotask to run, so the reported url reflects the click. Uses the
// native clock so a paused virtual clock cannot stall it.
await hwatuSleep(0);
return {{ clicked: matched, url: location.href }};"#,
        native = NATIVE_TIME_JS,
    );
    eval_with(daemon, id, js, timeout_ms, NavPolicy::Success, reply);
}

/// Type into an input/textarea/select/contenteditable. Values go
/// through the native setter so framework-controlled inputs (React)
/// observe the change, followed by input/change events. `enter`
/// presses Enter, which submits the enclosing form when the page
/// leaves the keydown unhandled.
#[allow(clippy::too_many_arguments)]
pub fn type_text(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
    selector: Option<String>,
    nth: Option<u32>,
    contains: Option<String>,
    ref_idx: Option<u32>,
    text: String,
    clear: bool,
    enter: bool,
    timeout_ms: Option<u64>,
    reply: Reply,
) {
    let prelude = match target_prelude(selector.as_deref(), nth, contains.as_deref(), ref_idx) {
        Ok(p) => p,
        Err(resp) => return OnceReply::new(reply).send(*resp),
    };
    let js = format!(
        r#"{native}{prelude}
const text = {text};
const clear = {clear};
const enter = {enter};
el.scrollIntoView({{ block: 'center', behavior: 'instant' }});
if (el.focus) el.focus();
const fire = (t) => el.dispatchEvent(new Event(t, {{ bubbles: true }}));
if (el instanceof HTMLSelectElement) {{
  const opt = [...el.options].find(o => o.value === text || o.textContent.trim() === text);
  if (!opt) throw new Error(`no <option> matching ${{JSON.stringify(text)}} (values: ${{[...el.options].map(o => o.value).slice(0, 20).join(', ')}})`);
  el.value = opt.value;
  fire('input'); fire('change');
}} else if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) {{
  const proto = el instanceof HTMLInputElement ? HTMLInputElement.prototype : HTMLTextAreaElement.prototype;
  const setter = Object.getOwnPropertyDescriptor(proto, 'value').set;
  setter.call(el, clear ? text : el.value + text);
  fire('input'); fire('change');
}} else if (el.isContentEditable) {{
  if (clear) el.textContent = '';
  el.dispatchEvent(new InputEvent('beforeinput', {{ bubbles: true, cancelable: true, inputType: 'insertText', data: text }}));
  document.execCommand ? document.execCommand('insertText', false, text) : el.textContent += text;
  fire('input');
}} else {{
  throw new Error(`element <${{el.tagName.toLowerCase()}}> is not typeable (need input, textarea, select, or contenteditable)`);
}}
if (enter) {{
  const key = {{ bubbles: true, cancelable: true, key: 'Enter', code: 'Enter', keyCode: 13, which: 13 }};
  const handled = !el.dispatchEvent(new KeyboardEvent('keydown', key));
  el.dispatchEvent(new KeyboardEvent('keyup', key));
  if (!handled && el.form) el.form.requestSubmit ? el.form.requestSubmit() : el.form.submit();
}}
await hwatuSleep(0);
const value = el.value !== undefined ? el.value : el.textContent;
return {{ typed: matched, value: String(value).slice(0, 200), url: location.href }};"#,
        native = NATIVE_TIME_JS,
        text = js_string(&text),
        clear = clear,
        enter = enter,
    );
    eval_with(daemon, id, js, timeout_ms, NavPolicy::Success, reply);
}

/// Read a window's console/error/network capture buffer. Synchronous
/// (the buffer lives daemon-side), but routed here for target
/// resolution. Works on suspended windows: the buffer outlives the
/// page.
pub fn console(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
    clear: bool,
    limit: Option<usize>,
) -> Response {
    match resolve(daemon, id) {
        Ok(win) => {
            let entries = win.console.read(clear, limit);
            Response::value(serde_json::to_value(entries).unwrap_or_default())
        }
        Err(resp) => *resp,
    }
}

/// JSON string literal, which is also a valid JS string literal.
fn js_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// Standard base64 (RFC 4648, with padding). Hand-rolled to keep
/// hwatud dependency-light; ~20 lines beats a crate for one call site.
fn base64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Minimal extension → MIME map for common upload fixtures. Unknown
/// extensions get `application/octet-stream`, which pages rarely gate.
fn mime_from_extension(path: &str) -> &'static str {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("pdf") => "application/pdf",
        Some("txt") | Some("log") => "text/plain",
        Some("md") => "text/markdown",
        Some("csv") => "text/csv",
        Some("json") => "application/json",
        Some("xml") => "application/xml",
        Some("html") | Some("htm") => "text/html",
        Some("zip") => "application/zip",
        Some("mp4") => "video/mp4",
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::{base64, challenge_detect_js, challenge_wait_js};

    #[test]
    fn base64_matches_rfc_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn challenge_detect_js_reports_structured_manual_state() {
        let js = challenge_detect_js();
        for field in [
            "status",
            "challenge_type",
            "confidence",
            "evidence",
            "actionable",
            "manual_required",
            "elapsed_ms",
        ] {
            assert!(js.contains(field), "missing structured field {field}");
        }
        for signal in [
            "turnstile",
            "hcaptcha",
            "recaptcha",
            "checking if the site connection is secure",
        ] {
            assert!(js.contains(signal), "missing challenge signal {signal}");
        }
    }

    #[test]
    fn challenge_wait_js_times_out_to_manual_required_not_bypass() {
        let js = challenge_wait_js(2500);
        assert!(js.contains("const timeoutMs = 2500"));
        assert!(js.contains("manual_required"));
        assert!(js.contains("solve it in the browser"));
        assert!(!js.contains("2captcha"));
        assert!(!js.contains("anti-captcha"));
        assert!(!js.contains("capsolver"));
    }
}
