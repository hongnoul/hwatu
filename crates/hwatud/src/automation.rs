// SPDX-License-Identifier: MIT
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
use hwatu_ipc::{LoadStage, OpenMode, PressKey, Response, Viewport};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use webkit6::prelude::*;

/// Deferred response writer, callable exactly once.
pub type Reply = Box<dyn FnOnce(Response)>;

/// Send one response through a fresh once-guard. For modules
/// (coverage.rs) that need the early-error path without borrowing the
/// OnceReply internals.
pub fn send_once(reply: Reply, response: Response) {
    OnceReply::new(reply).send(response);
}

/// Wrap a one-shot reply into a callable that tolerates multiple
/// invocations (first wins). The coverage verbs' orchestration
/// (try_until, fork) share it across closures.
pub fn once(reply: Reply) -> Rc<dyn Fn(Response)> {
    let guard = OnceReply::new(reply);
    Rc::new(move |response| guard.send(response))
}

/// Run `done` once window `id` finishes loading (or `timeout_ms`
/// expires, whichever first — done runs either way; the caller
/// decides what "loaded enough" means afterwards).
pub fn after_load(daemon: &Rc<Daemon>, id: u64, timeout_ms: Option<u64>, done: Box<dyn FnOnce()>) {
    let done = Rc::new(RefCell::new(Some(done)));
    let fire = {
        let done = done.clone();
        move || {
            if let Some(f) = done.borrow_mut().take() {
                f();
            }
        }
    };
    let reply: Reply = Box::new({
        let fire = fire.clone();
        move |_| fire()
    });
    wait_load(
        daemon,
        Some(id),
        hwatu_ipc::LoadStage::Settled,
        timeout_ms,
        reply,
    );
}

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
pub(crate) const NATIVE_TIME_JS: &str = include_str!("../../../page-scripts/native-time.js");

/// Pick the target window: explicit id, else the focused window, else
/// the last window an automation command targeted (if still open),
/// else the only window. Every successful resolution records the
/// target, so an agent that opened or drove a window keeps addressing
/// it without repeating `id` even when nothing has WM focus (the
/// normal state for background/headless verification flows).
/// Genuine ambiguity is still an error rather than a guess: an agent
/// driving the wrong window is worse than a retry with an id.
pub(crate) fn resolve(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
) -> Result<Rc<BrowserWindow>, Box<Response>> {
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
pub(crate) fn live_view(win: &Rc<BrowserWindow>) -> Result<webkit6::WebView, Box<Response>> {
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

    let source = match plan_eval_source(&js) {
        Ok(source) => source,
        Err(message) => {
            unwire();
            return reply.send(Response::err(format!(
                "eval failed: SyntaxError: {message}"
            )));
        }
    };
    view.call_async_javascript_function(
        &source,
        None,
        None,
        None,
        Some(&cancellable),
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
                        let msg = e.to_string();
                        // WebKit's serializer refuses values JSON can't
                        // express (DOM nodes, functions, symbols, window)
                        // with an opaque "Unsupported result type". Name
                        // the fix, or an agent reads this as a wedged
                        // page and starts re-opening windows.
                        let hint = if msg.contains("Unsupported result type") {
                            "; the script's completion value is not JSON-serializable \
                             (DOM node, function, symbol, or window). Return plain \
                             data instead, e.g. el.textContent rather than el, or \
                             end a statement-style body with an explicit `return`"
                        } else {
                            ""
                        };
                        reply.send(Response::err(format!("eval failed: {msg}{hint}")));
                    }
                }
            }
        },
    );
}

/// Decide the one source string an eval runs in the page, or reject
/// bad syntax with the parser's message.
///
/// An earlier design probed the expression form by *compiling in the
/// page* (an API-world script that parse-fails for every function
/// body). WebKit reports that parse failure to the page as a
/// cross-origin-masked `window` error event, so each probe miss
/// buffered a spurious `{kind: "exception", text: "Script error."}`
/// console entry: the resize path's `return {...}` viewport
/// measurement emitted one per measure eval, polluting every resize
/// and multi-viewport sweep reply (issue #6). Parsing daemon-side
/// (same JSC engine, so exact dialect parity with the page) keeps
/// failed candidates out of the page entirely, so `hwatu console`
/// only ever reports the page's own errors, and saves the probe's
/// page round trip.
///
/// The form rules are unchanged: an *expression* runs as `return
/// (expr)` (so `document.title` just works); a function body with an
/// explicit `return` or `await` runs as-is; a return-less
/// multi-statement body routes through indirect eval, which answers
/// with the completion value of the last statement the way a REPL
/// would (`await` cannot take that route: eval code is not an async
/// context, so it would turn into a SyntaxError).
fn plan_eval_source(js: &str) -> Result<String, String> {
    // Trailing semicolons are meaningless on an expression but would
    // break the `return ( ... )` wrapping; strip them for the
    // expression form.
    let trimmed = js.trim().trim_end_matches(';').trim_end();
    let expr = format!("return (\n{trimmed}\n);");
    if js_syntax_check(&wrap_async_body(&expr)).is_ok() {
        return Ok(expr);
    }
    let body = if js.contains("return") || js.contains("await") {
        js.to_string()
    } else {
        format!(
            "return (0, eval)({});",
            serde_json::to_string(js).expect("string serializes")
        )
    };
    js_syntax_check(&wrap_async_body(&body))?;
    Ok(body)
}

/// The shape `call_async_javascript_function` compiles a body into
/// (an async function wrapper), so daemon-side parse verdicts match
/// what the page's compile would say about the same body.
fn wrap_async_body(body: &str) -> String {
    format!("async function __hwatu_probe() {{\n{body}\n}}")
}

/// Parse-only syntax check in a daemon-local JSC context (the same
/// engine the page uses, so the dialect matches exactly).
/// `jsc_context_check_syntax` never executes code, so hostile input
/// that escapes the wrapper braces still cannot run in the daemon;
/// the context is reused (thread_local) because creating one costs
/// ~1 ms while a check costs microseconds.
fn js_syntax_check(code: &str) -> Result<(), String> {
    use glib::translate::{from_glib_full, ToGlibPtr};
    use webkit6::javascriptcore as jsc;
    thread_local! {
        static CTX: jsc::Context = jsc::Context::new();
    }
    CTX.with(|ctx| {
        let mut exception: *mut jsc::ffi::JSCException = std::ptr::null_mut();
        // SAFETY: a plain FFI parse call; every pointer is owned by
        // this frame, and the nullable exception out-pointer is
        // adopted into a managed Option immediately. The safe binding
        // is unusable here: it asserts the exception is non-null,
        // which is exactly the success case.
        let result = unsafe {
            jsc::ffi::jsc_context_check_syntax(
                ctx.to_glib_none().0,
                code.to_glib_none().0,
                code.len() as _,
                jsc::ffi::JSC_CHECK_SYNTAX_MODE_SCRIPT,
                "hwatu-eval".to_glib_none().0,
                1,
                &mut exception,
            )
        };
        let exception: Option<jsc::Exception> = unsafe { from_glib_full(exception) };
        if result == jsc::ffi::JSC_CHECK_SYNTAX_RESULT_SUCCESS {
            return Ok(());
        }
        Err(exception
            .and_then(|e| e.message().map(|m| m.to_string()))
            .unwrap_or_else(|| "invalid syntax".into()))
    })
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
            LoadStage::Settled | LoadStage::Quiescent => !view.is_loading(),
            LoadStage::Committed | LoadStage::Dom => win.load_committed(),
        };
    if stage_reached {
        if until == LoadStage::Dom {
            // Committed but the DOM may still be parsing; confirm
            // readiness inside the page before releasing the caller.
            arm_timeout(reply.clone(), timeout_ms, "wait_load");
            return await_dom_ready(&view, move |_| reply.send(Response::window(win.info())));
        }
        if until == LoadStage::Quiescent {
            // Settled already; now wait out the hydration tail.
            arm_timeout(reply.clone(), timeout_ms, "wait_load");
            return await_dom_quiescent(&view, move |_| reply.send(Response::window(win.info())));
        }
        return reply.send(Response::window(win.info()));
    }
    arm_timeout(reply.clone(), timeout_ms, "wait_load");
    wire_load_stage(&view, win.clone(), until, move |_| {
        reply.send(Response::window(win.info()))
    });
}

/// Detect CAPTCHA / anti-bot challenge UI, and optionally wait for it to
/// clear. Agents solve challenges with the standard commands (`shot`,
/// `snapshot`, `click --trusted`, `type`); this command reports what is on
/// the page. With `wait`, a challenge still present at the deadline returns
/// a structured `challenge` stop so the agent can hand off through `show`.
/// It never calls solver services, injects answer tokens, or alters
/// browser fingerprints.
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
    let reply: Reply = if wait {
        Box::new(move |resp| match &resp {
            Response::Ok {
                value: Some(value), ..
            } if value.get("status").and_then(|s| s.as_str()) == Some("manual_required") => {
                reply(Response::stop(
                    "challenge still present; the agent could not clear it",
                    crate::stops::challenge_blocked(value),
                ))
            }
            _ => reply(resp),
        })
    } else {
        reply
    };
    // The in-page poll owns the deadline. The eval timeout must outlast it
    // or it fires first and the agent gets "eval timed out" instead of the
    // structured challenge stop.
    let eval_timeout = if wait {
        Some(ms.saturating_add(CHALLENGE_EVAL_GRACE_MS))
    } else {
        timeout_ms
    };
    eval_with(daemon, id, js, eval_timeout, NavPolicy::Success, reply);
}

const CHALLENGE_EVAL_GRACE_MS: u64 = 5_000;

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
  current.details = `challenge still present after ${{current.elapsed_ms}} ms`;
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

/// Shared with every engine backend: `page-scripts/challenge.js`.
const CHALLENGE_DETECTOR_JS: &str = include_str!("../../../page-scripts/challenge.js");

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

/// Release once the DOM stops mutating: no mutations for
/// `QUIESCENT_QUIET_MS`, bounded by `QUIESCENT_CAP_MS` so a page with
/// a ticking clock or animation loop cannot stall the caller forever.
/// This runs *after* Settled and is what turns "the load event fired"
/// into "the SPA finished hydrating what it was going to hydrate".
/// On cap expiry it still succeeds: the caller wanted "as stable as
/// it gets", and endless mutation is as stable as that page gets.
const QUIESCENT_QUIET_MS: u64 = 300;
const QUIESCENT_CAP_MS: u64 = 5_000;

fn await_dom_quiescent(view: &webkit6::WebView, done: impl FnOnce(&webkit6::WebView) + 'static) {
    let js = format!(
        r#"const QUIET = {quiet};
const CAP = {cap};
return await new Promise((resolve) => {{
  let last = performance.now();
  const started = last;
  const mo = new MutationObserver(() => {{ last = performance.now(); }});
  mo.observe(document.documentElement || document, {{
    childList: true, subtree: true, attributes: true, characterData: true,
  }});
  const tick = () => {{
    const now = performance.now();
    if (now - last >= QUIET) {{
      mo.disconnect();
      resolve({{ quiescent: true, waited_ms: Math.round(now - started) }});
    }} else if (now - started >= CAP) {{
      mo.disconnect();
      resolve({{ quiescent: false, waited_ms: Math.round(now - started),
                 note: 'DOM still mutating at cap; page may animate continuously' }});
    }} else {{
      setTimeout(tick, Math.min(QUIET / 3, 100));
    }}
  }};
  setTimeout(tick, Math.min(QUIET / 3, 100));
}});"#,
        quiet = QUIESCENT_QUIET_MS,
        cap = QUIESCENT_CAP_MS,
    );
    let view2 = view.clone();
    view.call_async_javascript_function(&js, None, None, None, gio::Cancellable::NONE, move |_| {
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
        LoadStage::Quiescent => {
            wire_load_settled(view, win, move |view| await_dom_quiescent(view, done))
        }
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
fn validate_inline_check_outputs(
    shot_data: bool,
    heatmap_data: bool,
    viewport_count: usize,
) -> Result<(), &'static str> {
    if shot_data && heatmap_data {
        return Err(
            "one check cannot return both inline screenshot and heatmap data; request them separately",
        );
    }
    if shot_data && viewport_count > 1 {
        return Err(
            "inline screenshots support one viewport per check; request viewport screenshots separately",
        );
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn check(
    daemon: &Rc<Daemon>,
    url: Option<String>,
    render: Option<String>,
    base: Option<String>,
    eval_js: Option<String>,
    shot: bool,
    shot_path: Option<String>,
    shot_data: bool,
    full: bool,
    baseline: Option<String>,
    baseline_data: Option<String>,
    tolerance: Option<u8>,
    min_region_px: Option<u32>,
    heatmap: Option<String>,
    heatmap_data: bool,
    until: LoadStage,
    keep: bool,
    timeout_ms: Option<u64>,
    viewports: Vec<Viewport>,
    baseline_dir: Option<String>,
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
    // Sweep flag coherence: per-size baselines only make sense with a
    // sweep, and a single-file baseline cannot cover N differently
    // sized frames (the union-canvas diff would report noise).
    if baseline_dir.is_some() && viewports.is_empty() {
        return reply.send(Response::err("`baseline_dir` requires `viewports`"));
    }
    if baseline.is_some() && !viewports.is_empty() {
        return reply.send(Response::err(
            "pass `baseline_dir` (per-size baselines), not `baseline`, with `viewports`",
        ));
    }
    if baseline_data.is_some() && !viewports.is_empty() {
        return reply.send(Response::err(
            "inline baseline data is not supported with viewport sweeps",
        ));
    }
    if baseline.is_some() && baseline_data.is_some() {
        return reply.send(Response::err(
            "baseline path and inline baseline data are mutually exclusive",
        ));
    }
    if let Err(error) = validate_inline_check_outputs(shot_data, heatmap_data, viewports.len()) {
        return reply.send(Response::err(error));
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
    // Sweeps load at the first requested size, so breakpoint-dependent
    // initial renders (and any JS that reads innerWidth once at load)
    // see the right viewport from the start. Pass 0 still runs the
    // measure-and-correct resize to confirm what the page sees.
    if let Some(first) = viewports.first() {
        win.resize_viewport(first.w, first.h);
    }

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

        // Multi-viewport sweep: run the pass per size sequentially on
        // this one loaded window, then reply once with everything.
        if !viewports.is_empty() {
            let sweep = Rc::new(Sweep {
                daemon: daemon.clone(),
                win_id,
                viewports: viewports.clone(),
                eval_js: eval_js.clone(),
                shot: shot || shot_path.is_some() || shot_data,
                shot_path: shot_path.clone(),
                shot_data,
                full,
                baseline_dir: baseline_dir.clone(),
                tolerance,
                min_region_px,
                heatmap: heatmap.clone(),
                heatmap_data,
                keep,
                timeout_ms,
                started,
                reply: reply.clone(),
                result: result.clone(),
                view: view.clone(),
                entries: RefCell::new(Vec::new()),
            });
            sweep_pass(sweep, 0);
            return;
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
                        Response::Err { message, .. } => serde_json::json!({ "error": message }),
                    };
                    finish();
                }),
            );
        }
        if shot || shot_path.is_some() || shot_data {
            pending.set(pending.get() + 1);
            let finish = finish.clone();
            let result = result.clone();
            screenshot(
                &daemon,
                Some(win_id),
                shot_path,
                full,
                shot_data,
                Box::new(move |resp| {
                    match resp {
                        Response::Ok { path: Some(p), .. } => {
                            result.borrow_mut()["shot"] = serde_json::json!(p);
                        }
                        Response::Ok {
                            data: Some(data), ..
                        } => {
                            result.borrow_mut()["shot_data"] = serde_json::json!(data);
                        }
                        Response::Err { message, .. } => {
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
                crate::verify::DiffOptions {
                    tolerance,
                    min_region_px,
                    heatmap: heatmap.clone(),
                    heatmap_data,
                    full,
                },
                Box::new(move |value| {
                    result.borrow_mut()["diff"] = match value {
                        Ok(v) => v,
                        Err(e) => serde_json::json!({ "error": e }),
                    };
                    finish();
                }),
            );
        }
        if let Some(encoded) = baseline_data {
            pending.set(pending.get() + 1);
            let finish = finish.clone();
            let result = result.clone();
            crate::verify::diff_against_baseline_data(
                &daemon,
                win_id,
                encoded,
                crate::verify::DiffOptions {
                    tolerance,
                    min_region_px,
                    heatmap,
                    heatmap_data,
                    full,
                },
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
            if until == LoadStage::Quiescent {
                // Load finished, but the hydration tail is exactly
                // what quiescent exists to wait out.
                await_dom_quiescent(&view, proceed);
            } else {
                let view = view.clone();
                glib::idle_add_local_once(move || proceed(&view));
            }
        } else {
            match until {
                LoadStage::Committed => {
                    let view = view.clone();
                    glib::idle_add_local_once(move || proceed(&view));
                }
                LoadStage::Dom => await_dom_ready(&view, proceed),
                LoadStage::Settled => wire_load_settled(&view, win.clone(), proceed),
                LoadStage::Quiescent => wire_load_settled(&view, win.clone(), move |view| {
                    await_dom_quiescent(view, proceed)
                }),
            }
        }
    } else {
        wire_load_stage(&view, win.clone(), until, proceed);
    }
}

/// State for one multi-viewport check sweep: the loaded window, the
/// per-pass options, and the accumulated per-viewport entries. The
/// window is resized between passes rather than reopened: measured
/// here (scripts/bench-viewports.sh, warm pool, local fixture with a
/// breakpoint, eval+shot per size), a 3-size sweep totals ~17-21 ms
/// (per-pass resize+eval+shot 1-11 ms) vs ~55-92 ms for 3 separate
/// checks, and on real pages a fresh load costs hundreds of ms per
/// size on top. One window also guarantees every size sees the same
/// document instance, not N racing loads, and adds no daemon memory.
struct Sweep {
    daemon: Rc<Daemon>,
    win_id: u64,
    viewports: Vec<Viewport>,
    eval_js: Option<String>,
    shot: bool,
    shot_path: Option<String>,
    shot_data: bool,
    full: bool,
    baseline_dir: Option<String>,
    tolerance: Option<u8>,
    min_region_px: Option<u32>,
    heatmap: Option<String>,
    heatmap_data: bool,
    keep: bool,
    timeout_ms: Option<u64>,
    started: std::time::Instant,
    reply: OnceReply,
    result: Rc<RefCell<serde_json::Value>>,
    view: webkit6::WebView,
    entries: RefCell<Vec<serde_json::Value>>,
}

/// Derive the per-size variant of a caller-supplied path: insert
/// `-<WxH>` before the extension (`shot.png` -> `shot-360x640.png`).
fn per_size_path(path: &str, label: &str) -> String {
    let p = std::path::Path::new(path);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("shot");
    let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("png");
    let name = format!("{stem}-{label}.{ext}");
    match p.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.join(name).to_string_lossy().into_owned(),
        _ => name,
    }
}

/// Run sweep pass `i`: resize the window to `viewports[i]`, let the
/// relayout land, then eval/shot/diff at that size and recurse to
/// `i + 1`. The final pass assembles the combined reply.
fn sweep_pass(sweep: Rc<Sweep>, i: usize) {
    if sweep.reply.is_spent() {
        return; // overall check timeout already answered
    }
    let Some(vp) = sweep.viewports.get(i).copied() else {
        return sweep_finish(sweep);
    };
    let pass_started = std::time::Instant::now();
    // resize() measures the resulting CSS viewport (and corrects
    // fractional-scale backends), so when its reply lands the page
    // has really relaid out at the new size; media queries and
    // resize handlers have run.
    let sweep2 = sweep.clone();
    crate::verify::resize(
        &sweep.daemon.clone(),
        Some(sweep.win_id),
        vp.w,
        vp.h,
        Box::new(move |resp| {
            let sweep = sweep2;
            if sweep.reply.is_spent() {
                return;
            }
            let label = vp.label();
            let entry = Rc::new(RefCell::new(serde_json::json!({ "size": label })));
            if let Response::Err { message, .. } = resp {
                entry.borrow_mut()["error"] =
                    serde_json::json!(format!("resize to {label} failed: {message}"));
                sweep.entries.borrow_mut().push(entry.borrow().clone());
                return sweep_pass(sweep, i + 1);
            }

            // Same concurrency shape as the single-viewport pass:
            // eval/shot/diff run in parallel, the last one advances
            // the sweep to the next size.
            let pending = Rc::new(Cell::new(1u32));
            let advance = {
                let sweep = sweep.clone();
                let entry = entry.clone();
                let pending = pending.clone();
                Rc::new(move || {
                    pending.set(pending.get() - 1);
                    if pending.get() > 0 || sweep.reply.is_spent() {
                        return;
                    }
                    entry.borrow_mut()["pass_ms"] =
                        serde_json::json!(pass_started.elapsed().as_millis() as u64);
                    sweep.entries.borrow_mut().push(entry.borrow().clone());
                    sweep_pass(sweep.clone(), i + 1);
                })
            };

            if let Some(js) = sweep.eval_js.clone() {
                pending.set(pending.get() + 1);
                let advance = advance.clone();
                let entry = entry.clone();
                eval(
                    &sweep.daemon,
                    Some(sweep.win_id),
                    js,
                    sweep.timeout_ms,
                    Box::new(move |resp| {
                        entry.borrow_mut()["eval"] = match resp {
                            Response::Ok { value, .. } => value.unwrap_or(serde_json::Value::Null),
                            Response::Err { message, .. } => {
                                serde_json::json!({ "error": message })
                            }
                        };
                        advance();
                    }),
                );
            }
            if sweep.shot {
                pending.set(pending.get() + 1);
                let advance = advance.clone();
                let entry = entry.clone();
                let path = sweep.shot_path.as_deref().map(|p| per_size_path(p, &label));
                screenshot(
                    &sweep.daemon,
                    Some(sweep.win_id),
                    path,
                    sweep.full,
                    sweep.shot_data,
                    Box::new(move |resp| {
                        match resp {
                            Response::Ok { path: Some(p), .. } => {
                                entry.borrow_mut()["shot"] = serde_json::json!(p);
                            }
                            Response::Ok {
                                data: Some(data), ..
                            } => {
                                entry.borrow_mut()["shot_data"] = serde_json::json!(data);
                            }
                            Response::Err { message, .. } => {
                                entry.borrow_mut()["shot"] =
                                    serde_json::json!({ "error": message });
                            }
                            _ => {}
                        }
                        advance();
                    }),
                );
            }
            if let Some(dir) = sweep.baseline_dir.as_deref() {
                pending.set(pending.get() + 1);
                let advance = advance.clone();
                let entry = entry.clone();
                let baseline = std::path::Path::new(dir)
                    .join(format!("{label}.png"))
                    .to_string_lossy()
                    .into_owned();
                let heatmap = sweep.heatmap.as_deref().map(|p| per_size_path(p, &label));
                crate::verify::diff_against_baseline(
                    &sweep.daemon,
                    sweep.win_id,
                    baseline,
                    crate::verify::DiffOptions {
                        tolerance: sweep.tolerance,
                        min_region_px: sweep.min_region_px,
                        heatmap,
                        heatmap_data: sweep.heatmap_data,
                        full: sweep.full,
                    },
                    Box::new(move |value| {
                        entry.borrow_mut()["diff"] = match value {
                            Ok(v) => v,
                            Err(e) => serde_json::json!({ "error": e }),
                        };
                        advance();
                    }),
                );
            }
            advance(); // release the base hold
        }),
    );
}

/// Assemble the sweep reply: console (whole sweep), title, timings,
/// and the per-viewport entries. Resets the window's viewport before
/// parking so a later plain check does not inherit the last size.
fn sweep_finish(sweep: Rc<Sweep>) {
    let console = sweep
        .daemon
        .windows
        .borrow()
        .get(&sweep.win_id)
        .map(|w| w.console.read(true, Some(20)))
        .unwrap_or_default();
    {
        let mut r = sweep.result.borrow_mut();
        if !console.is_empty() {
            r["console"] = serde_json::to_value(&console).unwrap_or_default();
        }
        r["viewports"] = serde_json::Value::Array(sweep.entries.borrow().clone());
    }
    let sweep2 = sweep.clone();
    sweep.view.evaluate_javascript(
        "document.title",
        None,
        None,
        gio::Cancellable::NONE,
        move |title| {
            let sweep = sweep2;
            {
                let mut r = sweep.result.borrow_mut();
                r["title"] = serde_json::json!(title
                    .ok()
                    .map(|v| v.to_str().to_string())
                    .unwrap_or_default());
                r["total_ms"] = serde_json::json!(sweep.started.elapsed().as_millis() as u64);
            }
            if let Some(win) = sweep.daemon.windows.borrow().get(&sweep.win_id).cloned() {
                if !sweep.keep {
                    win.reset_viewport();
                }
            }
            release_check_window(&sweep.daemon, sweep.win_id, sweep.keep);
            sweep
                .reply
                .send(Response::value(sweep.result.borrow().clone()));
        },
    );
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
    // Fresh windows open at about:blank, never a start page: the
    // caller's load replaces it immediately, and the stale-blank
    // machinery (`note_load_engaged`, the stage wiring) already knows
    // how to ignore a superseded about:blank load. A start-page load
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
    data: bool,
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

    let region = crate::vocab::SnapshotArea::from_full_page(full).to_webkit();
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
                    let encoded = gio::spawn_blocking(move || -> Result<Vec<u8>, String> {
                        let png = encode_png(&bytes, stride, width, height)?;
                        if data {
                            Ok(png)
                        } else {
                            std::fs::write(&path, png).map_err(|error| error.to_string())?;
                            Ok(Vec::new())
                        }
                    })
                    .await;
                    match encoded {
                        Ok(Ok(png)) if data => {
                            if png.len() > hwatu_ipc::INLINE_MAX_BYTES {
                                reply.send(Response::err(format!(
                                    "encoded screenshot is {} bytes; inline limit is {} bytes",
                                    png.len(),
                                    hwatu_ipc::INLINE_MAX_BYTES
                                )));
                            } else {
                                reply.send(Response::data(hwatu_ipc::base64::encode(&png)));
                            }
                        }
                        Ok(Ok(_)) => reply.send(Response::path(target.to_string_lossy())),
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
fn encode_png(rgba: &[u8], stride: usize, width: u32, height: u32) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(width as usize * height as usize * 2 + 64);
    let mut encoder = png::Encoder::new(&mut out, width, height);
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
    writer.finish().map_err(|e| e.to_string())?;
    Ok(out)
}

/// Set a file input through WebKit's native file chooser request. A synthetic
/// JS `File` looks selected to page code but is not reliably path-backed when
/// WebKit serializes a form submission, so upload bytes can disappear at the
/// exact point the application submits them.
pub fn upload(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
    selector: String,
    path: String,
    data: Option<String>,
    timeout_ms: Option<u64>,
    reply: Reply,
) {
    let (chooser_path, staged) = if let Some(encoded) = data {
        let max_encoded = hwatu_ipc::INLINE_MAX_BYTES.div_ceil(3) * 4;
        if encoded.len() > max_encoded {
            return OnceReply::new(reply).send(Response::err(format!(
                "upload data exceeds the {}-byte decoded limit",
                hwatu_ipc::INLINE_MAX_BYTES
            )));
        }
        let bytes = match hwatu_ipc::base64::decode(&encoded) {
            Ok(bytes) => bytes,
            Err(error) => {
                return OnceReply::new(reply).send(Response::err(format!(
                    "upload data is not valid base64: {error}"
                )));
            }
        };
        let staged = match crate::private_files::stage_upload(&path, &bytes) {
            Ok(staged) => staged,
            Err(error) => return OnceReply::new(reply).send(Response::err(error)),
        };
        (staged.path().to_string_lossy().into_owned(), Some(staged))
    } else {
        let file = match std::fs::File::open(&path) {
            Ok(file) => file,
            Err(e) => {
                return OnceReply::new(reply)
                    .send(Response::err(format!("cannot read {path}: {e}")));
            }
        };
        match file.metadata() {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => {
                return OnceReply::new(reply).send(Response::err(format!(
                    "cannot upload {path}: not a regular file"
                )));
            }
            Err(e) => {
                return OnceReply::new(reply)
                    .send(Response::err(format!("cannot inspect {path}: {e}")));
            }
        }
        (path, None)
    };

    let win = match resolve(daemon, id) {
        Ok(win) => win,
        Err(response) => return OnceReply::new(reply).send(*response),
    };
    if let Err(response) = live_view(&win) {
        return OnceReply::new(reply).send(*response);
    }
    if let Err(error) = win.arm_upload(chooser_path, staged) {
        return OnceReply::new(reply).send(Response::err(error));
    }

    let win_id = win.id;
    let cleanup = win.clone();
    let reply: Reply = Box::new(move |response| {
        cleanup.clear_pending_upload();
        reply(response);
    });
    eval(
        daemon,
        Some(win_id),
        upload_js(&selector),
        timeout_ms,
        reply,
    );
}

fn upload_js(selector: &str) -> String {
    format!(
        r#"const selector = {selector};
		const documents = [document];
		for (let i = 0; i < documents.length; i++) {{
	  for (const frame of documents[i].querySelectorAll('iframe,frame')) {{
	    try {{
	      const child = frame.contentDocument;
	      if (child && !documents.includes(child)) documents.push(child);
	    }} catch (_) {{}}
	  }}
	}}
	const el = documents.map(doc => doc.querySelector(selector)).find(Boolean);
	if (!el) throw new Error('no element matches selector: ' + selector);
		const view = el.ownerDocument.defaultView || window;
		if (!(el instanceof view.HTMLInputElement) || el.type !== 'file')
		  throw new Error('element is not an <input type=file>: ' + selector);
		// Clearing first makes selecting the same path twice observable in
		// WebKit, which otherwise suppresses the second input/change pair.
		el.value = '';
		return await new Promise((resolve, reject) => {{
		  el.addEventListener('change', () => {{
		    const file = el.files && el.files[0];
		    if (!file) return reject(new Error('file chooser completed without a file'));
		    resolve({{ uploaded: file.name, size: file.size, type: file.type }});
		  }}, {{ once: true }});
		  el.click();
		}});"#,
        selector = js_string(selector),
    )
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
        "{native}const selector = {selector};\nconst nth = {nth};\nconst contains = {contains};\nconst toY = {to_y};\nconst byPages = {by_pages};\n{SCROLL_JS}",
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
pub(crate) fn json_or_null(s: Option<&str>) -> String {
    s.map_or("null".into(), js_string)
}

// Shared by one-shot and resident assertions. Keep this as plain page-side
// JavaScript so both call sites apply exactly the same rendered-state rules.
// The caller owns stability sampling because one-shot expect polls inside one
// eval while expect --watch is sampled by the native GLib scheduler.
pub(crate) const VISIBILITY_INSPECTOR_JS: &str =
    include_str!("../../../page-scripts/visibility.js");

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
        "{native}{visibility_inspector}\nconst selector = {selector};\nconst nth = {nth};\nconst contains = {contains};\nconst wantText = {want_text};\nconst absent = {absent};\nconst wantVisible = {visible};\nconst deadline = {deadline};\n{EXPECT_JS}",
        native = NATIVE_TIME_JS,
        visibility_inspector = VISIBILITY_INSPECTOR_JS,
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

/// Install a resident assertion monitor. The page gets a MutationObserver
/// so DOM replacement marks the document dirty immediately, while native
/// GLib/WebKit callbacks do the actual scheduling and navigation
/// termination. Because the scheduler lives outside the page's virtual
/// clock, `hwatu clock pause` cannot stall the monitor.
#[allow(clippy::too_many_arguments)]
pub fn expect_watch(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
    selector: String,
    nth: Option<u32>,
    contains: Option<String>,
    text: Option<String>,
    absent: bool,
    visible: bool,
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
    let window_id = win.id;
    let spec = Rc::new(ExpectSpec::new(
        selector, nth, contains, text, absent, visible, window_id,
    ));
    let state = Rc::new(ExpectWatchState {
        daemon: daemon.clone(),
        view: view.clone(),
        spec,
        last_ok: Cell::new(None),
        seq: Cell::new(0),
        in_flight: Cell::new(false),
        done: Cell::new(false),
        source: RefCell::new(None),
        nav_handler: RefCell::new(None),
    });

    install_expect_observer(&view);
    arm_expect_navigation(&state);
    let source_state = state.clone();
    let source = glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        if source_state.done.get() {
            glib::ControlFlow::Break
        } else {
            tick_expect_watch(&source_state, false);
            glib::ControlFlow::Continue
        }
    });
    state.source.replace(Some(source));
    tick_expect_watch(&state, true);
    reply.send(Response::value(serde_json::json!({
        "watch": true,
        "window_id": window_id,
        "event": "expect"
    })));
}

struct ExpectSpec {
    selector: String,
    nth: u32,
    contains: Option<String>,
    text: Option<String>,
    absent: bool,
    visible: bool,
    window_id: u64,
}

impl ExpectSpec {
    fn new(
        selector: String,
        nth: Option<u32>,
        contains: Option<String>,
        text: Option<String>,
        absent: bool,
        visible: bool,
        window_id: u64,
    ) -> Self {
        Self {
            selector,
            nth: nth.unwrap_or(0),
            contains,
            text,
            absent,
            visible,
            window_id,
        }
    }
}

struct ExpectWatchState {
    daemon: Rc<Daemon>,
    view: webkit6::WebView,
    spec: Rc<ExpectSpec>,
    last_ok: Cell<Option<bool>>,
    seq: Cell<u64>,
    in_flight: Cell<bool>,
    done: Cell<bool>,
    source: RefCell<Option<glib::SourceId>>,
    nav_handler: RefCell<Option<glib::SignalHandlerId>>,
}

fn install_expect_observer(view: &webkit6::WebView) {
    view.evaluate_javascript(
        r#"(() => {
  if (window.__hwatuExpectObserver) return true;
  window.__hwatuExpectDirty = true;
  window.__hwatuExpectVersion = 0;
  const mark = () => { window.__hwatuExpectDirty = true; window.__hwatuExpectVersion++; };
  const obs = new MutationObserver(mark);
  obs.observe(document, { subtree: true, childList: true, attributes: true, characterData: true });
  window.addEventListener('DOMContentLoaded', mark, { capture: true });
  window.addEventListener('load', mark, { capture: true });
  window.__hwatuExpectObserver = obs;
  return true;
        })()"#,
        None,
        None,
        gio::Cancellable::NONE,
        |_| {},
    );
}

fn arm_expect_navigation(state: &Rc<ExpectWatchState>) {
    let weak = Rc::downgrade(state);
    let handler = state.view.connect_load_changed(move |view, event| {
        if event != webkit6::LoadEvent::Committed {
            return;
        }
        let Some(state) = weak.upgrade() else { return };
        if state.done.replace(true) {
            return;
        }
        if let Some(source) = state.source.borrow_mut().take() {
            source.remove();
        }
        let uri = view.uri().map(|s| s.to_string());
        emit_expect(
            &state,
            "navigation",
            None,
            serde_json::json!({ "terminal": true, "url": uri }),
        );
    });
    state.nav_handler.replace(Some(handler));
}

fn tick_expect_watch(state: &Rc<ExpectWatchState>, force: bool) {
    if state.done.get() || state.in_flight.replace(true) {
        return;
    }
    let js = expect_watch_js(&state.spec, force);
    let weak = Rc::downgrade(state);
    state.view.evaluate_javascript(
        &js,
        None,
        None,
        gio::Cancellable::NONE,
        move |res: Result<webkit6::javascriptcore::Value, glib::Error>| {
            let Some(state) = weak.upgrade() else { return };
            state.in_flight.set(false);
            if state.done.get() {
                return;
            }
            let value = match res {
                Ok(v) => jsc_to_json(&v),
                Err(e) => serde_json::json!({ "ok": false, "why": e.to_string() }),
            };
            if value.get("skipped").and_then(|v| v.as_bool()) == Some(true) {
                return;
            }
            // A visibility watch needs two matching native-scheduler samples.
            // Do not emit a false initial state while that evidence accumulates.
            if value.get("stabilizing").and_then(|v| v.as_bool()) == Some(true) {
                return;
            }
            let ok = value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
            let last = state.last_ok.get();
            if force || last != Some(ok) {
                state.last_ok.set(Some(ok));
                let phase = if last.is_none() { "initial" } else { "flip" };
                emit_expect(&state, phase, Some(ok), value);
            }
        },
    );
}

fn emit_expect(state: &ExpectWatchState, phase: &str, ok: Option<bool>, result: serde_json::Value) {
    let seq = state.seq.get() + 1;
    state.seq.set(seq);
    state.daemon.events.emit(
        "expect",
        Some(state.spec.window_id),
        serde_json::json!({
            "phase": phase,
            "ok": ok,
            "expect_seq": seq,
            "selector": &state.spec.selector,
            "nth": state.spec.nth,
            "contains": &state.spec.contains,
            "text": &state.spec.text,
            "absent": state.spec.absent,
            "visible": state.spec.visible,
            "result": result,
        }),
    );
}

fn expect_watch_js(spec: &ExpectSpec, force: bool) -> String {
    format!(
        r#"(() => {{
const force = {force};
window.__hwatuExpectDirty = false;
{visibility_inspector}
const selector = {selector};
const nth = {nth};
const contains = {contains};
const wantText = {want_text};
const absent = {absent};
const wantVisible = {visible};
let els = [...document.querySelectorAll(selector)];
const total = els.length;
if (contains !== null) els = els.filter(e => ((e.textContent || '') + ' ' + (e.value || '')).includes(contains));
const el = els[nth];
if (absent) {{
  if (els.length === 0) return {{ ok: true, matches: 0, version: window.__hwatuExpectVersion || 0 }};
  const t = (els[0].textContent || '').trim().slice(0, 120);
  return {{ ok: false, matches: els.length, why: `expected no match, found ${{els.length}} (first: <${{els[0].tagName.toLowerCase()}}> ${{JSON.stringify(t)}})`, version: window.__hwatuExpectVersion || 0 }};
}}
if (!el) {{
  const filt = contains === null ? '' : ` (${{els.length}} after contains filter)`;
  return {{ ok: false, matches: els.length, why: `no match: ${{total}} element(s) for selector${{filt}}, nth=${{nth}}`, version: window.__hwatuExpectVersion || 0 }};
}}
const actual = (el.textContent || '').trim();
if (wantText !== null && !actual.includes(wantText))
  return {{ ok: false, matches: els.length, tag: el.tagName.toLowerCase(), text: actual.slice(0, 120), why: `text mismatch: expected to contain ${{JSON.stringify(wantText)}}, got ${{JSON.stringify(actual.slice(0, 200))}}`, version: window.__hwatuExpectVersion || 0 }};
let visibility = null;
if (wantVisible) {{
  const sampleKey = JSON.stringify([selector, nth, contains]);
  window.__hwatuVisibilitySamples ||= {{}};
  visibility = hwatuInspectVisibility(el);
  if (visibility.why) {{
    delete window.__hwatuVisibilitySamples[sampleKey];
    return {{ ok: false, matches: els.length, tag: el.tagName.toLowerCase(), text: actual.slice(0, 120), why: `element matched but is not visible: ${{visibility.why}}`, visibility, version: window.__hwatuExpectVersion || 0 }};
  }}
  const sampleVersion = window.__hwatuExpectVersion || 0;
  const previousSample = window.__hwatuVisibilitySamples[sampleKey];
  if (!previousSample || previousSample.signature !== visibility.signature
      || previousSample.version !== sampleVersion) {{
    window.__hwatuVisibilitySamples[sampleKey] = {{ signature: visibility.signature, version: sampleVersion }};
    return {{ ok: false, stabilizing: true, matches: els.length, visibility, version: window.__hwatuExpectVersion || 0 }};
  }}
}}
return {{ ok: true, matches: els.length, tag: el.tagName.toLowerCase(), text: actual.slice(0, 120), ...(wantVisible ? {{ visibility }} : {{}}), version: window.__hwatuExpectVersion || 0 }};
}})()"#,
        force = force,
        visibility_inspector = VISIBILITY_INSPECTOR_JS,
        selector = js_string(&spec.selector),
        nth = spec.nth,
        contains = json_or_null(spec.contains.as_deref()),
        want_text = json_or_null(spec.text.as_deref()),
        absent = spec.absent,
        visible = spec.visible,
    )
}

/// Shared with every engine backend: `page-scripts/snapshot.js`.
pub(crate) const SNAPSHOT_JS: &str = include_str!("../../../page-scripts/snapshot.js");

/// Token-cheap page state: url, title, bounded visible text, and an
/// indexed list of interactable elements (links, buttons, inputs...).
/// The elements are remembered on `window.__hwatu_refs`, so a
/// follow-up click/type can target `ref: n` without a selector. The
/// designed alternative to screenshot-and-squint for agents.
pub fn snapshot(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
    diff: bool,
    rect: bool,
    budget: Option<usize>,
    timeout_ms: Option<u64>,
    reply: Reply,
) {
    let js = SNAPSHOT_JS.replace(
        "__HWATU_INCLUDE_RECTS__",
        if rect { "true" } else { "false" },
    );
    // Diff, budget and quarantine are engine-independent and shared with
    // the CLI (which finishes raw snapshots from other engines) through
    // `hwatu_snapshot::finish`.
    if !diff {
        return eval(
            daemon,
            id,
            js,
            timeout_ms,
            Box::new(move |resp| match resp {
                Response::Ok {
                    value: Some(raw), ..
                } => reply(Response::value(hwatu_snapshot::finish(&raw, None, budget))),
                other => reply(other),
            }),
        );
    }
    // Diff mode: pin the target window now so the baseline read and
    // the post-eval write hit the same window even if focus moves
    // while the script runs. The full snapshot script still executes
    // (so `window.__hwatu_refs` is refreshed and every `ref` in the
    // diff is a live handle); only the reply is reduced.
    let win = match resolve(daemon, id) {
        Ok(w) => w,
        Err(resp) => return reply(*resp),
    };
    let window_id = win.id;
    let daemon2 = daemon.clone();
    eval(
        daemon,
        Some(window_id),
        js,
        timeout_ms,
        Box::new(move |resp| {
            let Response::Ok {
                value: Some(raw), ..
            } = resp
            else {
                return reply(resp); // errors pass through untouched
            };
            // Re-fetch the window: it may have closed mid-eval.
            let Some(win) = daemon2.windows.borrow().get(&window_id).cloned() else {
                return reply(Response::err(format!(
                    "window {window_id} closed while the snapshot ran"
                )));
            };
            let previous = win.snapshot_baseline.replace(Some(raw.clone()));
            reply(Response::value(hwatu_snapshot::finish(
                &raw,
                Some(previous.as_ref()),
                budget,
            )))
        }),
    );
}

/// Shared with every engine backend: `page-scripts/target.js`.
pub(crate) const TARGET_JS: &str = include_str!("../../../page-scripts/target.js");
/// Shared with every engine backend: `page-scripts/click.js`.
pub(crate) const CLICK_JS: &str = include_str!("../../../page-scripts/click.js");
/// Shared with every engine backend: `page-scripts/type.js`.
pub(crate) const TYPE_JS: &str = include_str!("../../../page-scripts/type.js");

/// JS prelude that resolves a click/type target: by snapshot `ref` or
/// by selector + nth/contains (same disambiguation as scroll). Leaves
/// `el` (the element) and `matched` (the landing report) in scope, or
/// throws with an explanation an agent can act on.
pub(crate) fn target_prelude(
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
        "const selector = {selector};\nconst nth = {nth};\nconst contains = {contains};\nconst refIdx = {ref_idx};\n{TARGET_JS}",
        selector = json_or_null(selector),
        nth = nth.unwrap_or(0),
        contains = json_or_null(contains),
        ref_idx = ref_idx.map_or("null".into(), |v| v.to_string()),
    ))
}

fn trusted_target_source(
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
        r#"const resolver = window.__hwatuTrustedResolve;
if (!resolver) throw new Error('trusted input resolver is not installed; reload the page after upgrading hwatud');
return await resolver({selector}, {nth}, {contains}, {ref_idx});"#,
        selector = json_or_null(selector),
        nth = nth.unwrap_or(0),
        contains = json_or_null(contains),
        ref_idx = ref_idx.map_or("null".into(), |v| v.to_string()),
    ))
}

fn trusted_report(
    action: &str,
    target: crate::trusted_input::TargetPoint,
    input: crate::trusted_input::InputReport,
    url: Option<String>,
) -> Response {
    Response::value(serde_json::json!({
        action: target.matched,
        "trusted": true,
        "frame": target.frame,
        "iframe": target.iframe,
        "url": url.unwrap_or_default(),
        "input": {
            "backend": input.backend,
            "surface_x": input.x,
            "surface_y": input.y,
            "surface_size": [input.extent_width, input.extent_height],
        }
    }))
}

/// What a trusted injection does with the resolved target point.
enum TrustedAction {
    Click,
    Paste,
    Type {
        text: String,
        clear: bool,
        enter: bool,
    },
}

fn trusted_input_mode_error(mode: OpenMode) -> Option<&'static str> {
    match mode {
        OpenMode::Normal => None,
        OpenMode::Background | OpenMode::Headless => Some(
            "trusted input on this engine is delivered through the compositor and needs a \
             shown window: call `hwatu show <id>` first, or use a standard (untrusted) click",
        ),
    }
}

/// Shared driver for trusted click/type. Sequence, all on the GTK main
/// thread (the injection itself talks to the compositor over a private
/// Wayland connection, which is fine to block on for a few ms):
///
/// 1. `prepare`: present + fullscreen the window. Fullscreen makes
///    surface coords == output coords, sidestepping the "compositor
///    won't tell us where the window is" problem entirely.
/// 2. Wait until the surface actually covers the monitor (configure
///    round-trip), plus a relayout settle.
/// 3. Resolve the target element's viewport point in the page (the
///    all-frames resolver pierces cross-origin iframes via postMessage
///    from injected user scripts).
/// 4. Map to surface coords and inject through zwlr-virtual-pointer /
///    wtype. The events arrive from the compositor, so the page sees
///    `isTrusted: true` in whatever frame owns the point.
/// 5. Restore the window's previous fullscreen/visibility state.
#[allow(clippy::too_many_arguments)]
fn trusted_input_run(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
    selector: Option<String>,
    nth: Option<u32>,
    contains: Option<String>,
    ref_idx: Option<u32>,
    action: TrustedAction,
    point: Option<(f64, f64)>,
    timeout_ms: Option<u64>,
    reply: Reply,
) {
    let what: &'static str = match action {
        TrustedAction::Click => "trusted click",
        TrustedAction::Paste => "trusted paste",
        TrustedAction::Type { .. } => "trusted type",
    };
    let reply = OnceReply::new(reply);
    let win = match resolve(daemon, id) {
        Ok(w) => w,
        Err(resp) => return reply.send(*resp),
    };
    if let Some(message) = trusted_input_mode_error(win.mode()) {
        return reply.send(Response::err(format!("{what} refused: {message}")));
    }
    let view = match live_view(&win) {
        Ok(v) => v,
        Err(resp) => return reply.send(*resp),
    };
    let js = match point {
        // A fixed point needs no resolution; return it in the resolver's shape.
        Some((x, y)) => format!(
            "return {{ x: {x}, y: {y}, viewportWidth: innerWidth, viewportHeight: innerHeight, \
             frame: 'point', matched: {{ at: [{x}, {y}], tag: (document.elementFromPoint({x}, {y}) || {{}}).tagName?.toLowerCase() || null }} }};"
        ),
        None => match trusted_target_source(selector.as_deref(), nth, contains.as_deref(), ref_idx) {
            Ok(js) => js,
            Err(resp) => return reply.send(*resp),
        },
    };
    let source = match plan_eval_source(&js) {
        Ok(source) => source,
        Err(message) => {
            return reply.send(Response::err(format!(
                "{what} target eval failed: SyntaxError: {message}"
            )))
        }
    };
    arm_timeout(reply.clone(), timeout_ms, what);

    let session = crate::trusted_input::prepare(&win);
    // Any exit past this point must restore the window; wrap the reply
    // so sending it always finishes the session first.
    let reply = {
        let session = session.clone();
        let inner = reply;
        OnceReply::new(Box::new(move |resp| {
            session.finish();
            inner.send(resp);
        }))
    };

    let fullscreen_wait_ms = timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS).clamp(500, 5_000);
    let win_for_wait = win.clone();
    crate::trusted_input::when_fullscreen(&win_for_wait, fullscreen_wait_ms, move |ready| {
        if let Err(e) = ready {
            return reply.send(Response::err(format!("{what} failed: {e}")));
        }
        if reply.is_spent() {
            session.finish();
            return;
        }
        let view_for_cb = view.clone();
        view.call_async_javascript_function(
            &source,
            None,
            None,
            None,
            gio::Cancellable::NONE,
            move |result| {
                if reply.is_spent() {
                    session.finish();
                    return;
                }
                let target = match result
                    .map(|value| jsc_to_json(&value))
                    .map_err(|e| e.to_string())
                    .and_then(|json| {
                        serde_json::from_value::<crate::trusted_input::TargetPoint>(json)
                            .map_err(|e| e.to_string())
                    }) {
                    Ok(target) => target,
                    Err(e) => {
                        return reply.send(Response::err(format!("{what} target failed: {e}")))
                    }
                };
                // Injection is async: it interleaves virtual-pointer
                // probes with page readbacks (calibration) on the GTK
                // main loop.
                glib::spawn_future_local(async move {
                    let injected = match &action {
                        TrustedAction::Click => {
                            crate::trusted_input::inject_click(&win, &view_for_cb, &target).await
                        }
                        TrustedAction::Paste => {
                            crate::trusted_input::inject_paste(&win, &view_for_cb, &target).await
                        }
                        TrustedAction::Type { text, clear, enter } => {
                            crate::trusted_input::inject_type(
                                &win,
                                &view_for_cb,
                                &target,
                                text,
                                *clear,
                                *enter,
                            )
                            .await
                        }
                    };
                    let input = match injected {
                        Ok(report) => report,
                        Err(e) => return reply.send(Response::err(format!("{what} failed: {e}"))),
                    };
                    let url = view_for_cb.uri().map(|u| u.to_string());
                    let done = match action {
                        TrustedAction::Click => "clicked",
                        TrustedAction::Paste => "pasted",
                        TrustedAction::Type { .. } => "typed",
                    };
                    // Give the compositor round-trip + page handlers a
                    // beat before restoring the window and reporting.
                    glib::timeout_future(std::time::Duration::from_millis(120)).await;
                    reply.send(trusted_report(done, target, input, url));
                });
            },
        );
    });
}

/// Click an element with a real pointer-event sequence at the
/// element's center (pages listening on pointerdown/mousedown see the
/// same shape as a human click). Reports what was hit.
///
/// Unless `confirm` is set, the target is first classified by
/// [`crate::stops::IRREVERSIBLE_JS`]; a payment, send, or delete control
/// returns a structured `irreversible_action` stop and is not clicked.
#[allow(clippy::too_many_arguments)]
pub fn click(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
    selector: Option<String>,
    nth: Option<u32>,
    contains: Option<String>,
    ref_idx: Option<u32>,
    trusted: bool,
    confirm: bool,
    timeout_ms: Option<u64>,
    reply: Reply,
) {
    if confirm {
        return click_unguarded(
            daemon, id, selector, nth, contains, ref_idx, trusted, timeout_ms, reply,
        );
    }
    let prelude = match target_prelude(selector.as_deref(), nth, contains.as_deref(), ref_idx) {
        Ok(p) => p,
        Err(resp) => return OnceReply::new(reply).send(*resp),
    };
    let probe = format!(
        "{prelude}\nreturn {{ irreversible: {classifier} }};",
        classifier = crate::stops::IRREVERSIBLE_JS.trim()
    );
    let daemon2 = daemon.clone();
    eval_with(
        daemon,
        id,
        probe,
        timeout_ms,
        NavPolicy::Error,
        Box::new(move |resp| {
            let hit = match &resp {
                Response::Ok {
                    value: Some(value), ..
                } => value.get("irreversible").filter(|v| !v.is_null()).cloned(),
                // A probe failure (no match, bad ref) is reported by the
                // click itself with its usual message.
                _ => None,
            };
            match hit {
                Some(hit) => {
                    let (message, stop) = crate::stops::irreversible_action(&hit);
                    reply(Response::stop(message, stop));
                }
                None => click_unguarded(
                    &daemon2, id, selector, nth, contains, ref_idx, trusted, timeout_ms, reply,
                ),
            }
        }),
    );
}

/// Trusted click at a top-viewport CSS point (`click --trusted --at x,y`).
/// For targets with no selector, such as image regions inside a
/// cross-origin challenge frame. The irreversible guard classifies the
/// element at the point when it is in the top document; inside a
/// cross-origin frame the element is not readable and the click proceeds.
pub fn click_at(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
    x: f64,
    y: f64,
    confirm: bool,
    timeout_ms: Option<u64>,
    reply: Reply,
) {
    if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 {
        return OnceReply::new(reply).send(Response::err("click --at needs non-negative x,y"));
    }
    let probe = format!(
        "const el = document.elementFromPoint({x}, {y});\n\
         if (!el) throw new Error('no element at {x},{y} (outside the viewport?)');\n\
         const inFrame = el.tagName === 'IFRAME' || el.tagName === 'FRAME';\n\
         return {{ irreversible: inFrame ? null : {classifier} }};",
        classifier = crate::stops::IRREVERSIBLE_JS.trim()
    );
    let daemon2 = daemon.clone();
    eval_with(
        daemon,
        id,
        probe,
        timeout_ms,
        NavPolicy::Error,
        Box::new(move |resp| {
            let value = match resp {
                Response::Ok {
                    value: Some(value), ..
                } => value,
                other => return reply(other),
            };
            if !confirm {
                if let Some(hit) = value.get("irreversible").filter(|v| !v.is_null()) {
                    let (message, stop) = crate::stops::irreversible_action(hit);
                    return reply(Response::stop(message, stop));
                }
            }
            trusted_input_run(
                &daemon2,
                id,
                None,
                None,
                None,
                None,
                TrustedAction::Click,
                Some((x, y)),
                timeout_ms,
                reply,
            );
        }),
    );
}

#[allow(clippy::too_many_arguments)]
fn click_unguarded(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
    selector: Option<String>,
    nth: Option<u32>,
    contains: Option<String>,
    ref_idx: Option<u32>,
    trusted: bool,
    timeout_ms: Option<u64>,
    reply: Reply,
) {
    let prelude = match target_prelude(selector.as_deref(), nth, contains.as_deref(), ref_idx) {
        Ok(p) => p,
        Err(resp) => return OnceReply::new(reply).send(*resp),
    };
    if trusted {
        return trusted_input_run(
            daemon,
            id,
            selector,
            nth,
            contains,
            ref_idx,
            TrustedAction::Click,
            None,
            timeout_ms,
            reply,
        );
    }
    let js = format!("{native}{prelude}\n{CLICK_JS}", native = NATIVE_TIME_JS,);
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
    trusted: bool,
    clear: bool,
    enter: bool,
    timeout_ms: Option<u64>,
    reply: Reply,
) {
    let prelude = match target_prelude(selector.as_deref(), nth, contains.as_deref(), ref_idx) {
        Ok(p) => p,
        Err(resp) => return OnceReply::new(reply).send(*resp),
    };
    if trusted {
        return trusted_input_run(
            daemon,
            id,
            selector,
            nth,
            contains,
            ref_idx,
            TrustedAction::Type { text, clear, enter },
            None,
            timeout_ms,
            reply,
        );
    }
    let js = format!(
        "{native}{prelude}\nconst text = {text};\nconst clear = {clear};\nconst enter = {enter};\n{TYPE_JS}",
        native = NATIVE_TIME_JS,
        text = js_string(&text),
        clear = clear,
        enter = enter,
    );
    eval_with(daemon, id, js, timeout_ms, NavPolicy::Success, reply);
}

pub(crate) const PRESS_JS: &str = include_str!("../../../page-scripts/press.js");
/// Shared with every engine backend: `page-scripts/expect.js`.
pub(crate) const EXPECT_JS: &str = include_str!("../../../page-scripts/expect.js");
/// Shared with every engine backend: `page-scripts/scroll.js`.
pub(crate) const SCROLL_JS: &str = include_str!("../../../page-scripts/scroll.js");

fn press_js(key: PressKey) -> String {
    format!(
        "{NATIVE_TIME_JS}\n{}",
        PRESS_JS.replace("__HWATU_KEY__", &js_string(key.as_str()))
    )
}

/// Press a page-local key through DOM focus and activation semantics. Unlike
/// trusted compositor input, this works on headless windows and never presents
/// or focuses the native toplevel.
pub fn press(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
    key: PressKey,
    timeout_ms: Option<u64>,
    reply: Reply,
) {
    // WebKit only applies the :focus / :focus-visible pseudo-classes while
    // its WebView owns GTK focus.  A headless window is deliberately never
    // mapped, so JavaScript element.focus() alone updates activeElement but
    // leaves those pseudo-classes false and screenshots omit focus rings.
    // Grabbing widget focus is compositor-local: it does not map or present
    // the toplevel, but it gives WebKit the page-focus state its CSS engine
    // needs before PRESS_JS advances the active element.
    if key == PressKey::Tab {
        if let Ok(view) = resolve(daemon, id).and_then(|w| live_view(&w)) {
            view.grab_focus();
        }
    }
    eval_with(
        daemon,
        id,
        press_js(key),
        timeout_ms,
        NavPolicy::Success,
        reply,
    );
}

/// Paste from the compositor clipboard into an element with native trusted
/// input. This intentionally has no JS fallback: clipboard pasting must happen
/// through the compositor while the trusted input session owns focus.
#[allow(clippy::too_many_arguments)]
pub fn paste(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
    selector: Option<String>,
    nth: Option<u32>,
    contains: Option<String>,
    ref_idx: Option<u32>,
    timeout_ms: Option<u64>,
    reply: Reply,
) {
    trusted_input_run(
        daemon,
        id,
        selector,
        nth,
        contains,
        ref_idx,
        TrustedAction::Paste,
        None,
        timeout_ms,
        reply,
    );
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

/// Read a window's structured network request log. Synchronous (the
/// buffer lives daemon-side), but routed here for target resolution.
/// Works on suspended windows: the buffer outlives the page.
pub fn net(daemon: &Rc<Daemon>, id: Option<u64>, clear: bool, limit: Option<usize>) -> Response {
    match resolve(daemon, id) {
        Ok(win) => {
            let entries = win.net.read(clear, limit);
            Response::value(serde_json::to_value(entries).unwrap_or_default())
        }
        Err(resp) => *resp,
    }
}

/// JSON string literal, which is also a valid JS string literal.
pub(crate) fn js_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

#[cfg(test)]
mod tests {
    use super::{
        challenge_detect_js, challenge_wait_js, expect_watch_js, plan_eval_source, press_js,
        trusted_input_mode_error, upload_js, validate_inline_check_outputs, ExpectSpec, SCROLL_JS,
        VISIBILITY_INSPECTOR_JS,
    };
    use hwatu_ipc::{OpenMode, PressKey};

    #[test]
    fn upload_activates_native_chooser_instead_of_assigning_synthetic_files() {
        let js = upload_js("input[type=file]");
        assert!(js.contains("el.addEventListener('change'"));
        assert!(js.contains("el.click()"));
        assert!(js.contains("const file = el.files && el.files[0]"));
        assert!(!js.contains("DataTransfer"));
        assert!(!js.contains("el.files ="));
    }

    #[test]
    fn inline_check_outputs_never_aggregate_past_one_artifact() {
        assert!(validate_inline_check_outputs(true, false, 1).is_ok());
        assert!(validate_inline_check_outputs(false, true, 0).is_ok());
        assert!(validate_inline_check_outputs(true, true, 0).is_err());
        assert!(validate_inline_check_outputs(true, false, 2).is_err());
    }

    #[test]
    fn trusted_input_never_promotes_non_normal_windows() {
        assert_eq!(trusted_input_mode_error(OpenMode::Normal), None);
        for mode in [OpenMode::Background, OpenMode::Headless] {
            let message = trusted_input_mode_error(mode).expect("mode must be refused");
            assert!(message.contains("hwatu show"));
        }
    }

    #[test]
    fn tab_press_uses_page_local_focus_traversal() {
        let js = press_js(PressKey::Tab);
        assert!(js.contains("const key = \"Tab\""));
        assert!(js.contains("createTreeWalker"));
        assert!(js.contains("el.shadowRoot"));
        assert!(js.contains("el.contentDocument"));
        assert!(js.contains("focused.focus()"));
        assert!(js.contains("current + 1"));
        assert!(!js.contains("window.present"));
    }

    #[test]
    fn enter_press_dispatches_keys_and_activates_focused_element() {
        let js = press_js(PressKey::Enter);
        assert!(js.contains("const key = \"Enter\""));
        assert!(js.contains("'keydown'"));
        assert!(js.contains("'keypress'"));
        assert!(js.contains("'keyup'"));
        assert!(js.contains("active.click()"));
        assert!(js.contains("active.form.requestSubmit()"));
        assert!(js.contains("await hwatuSleep(0)"));
    }

    #[test]
    fn escape_press_runs_dialog_cancel_semantics() {
        let js = press_js(PressKey::Escape);
        assert!(js.contains("const key = \"Escape\""));
        assert!(js.contains("key === 'Escape' ? 27"));
        assert!(js.contains("querySelectorAll('dialog[open]')"));
        assert!(js.contains("new view.Event('cancel', { cancelable: true })"));
        assert!(js.contains("if (cancelAllowed && dialog.open) dialog.close()"));
    }

    #[test]
    fn arrow_press_preserves_focused_horizontal_scroll_default() {
        for (key, name, code) in [
            (PressKey::ArrowLeft, "ArrowLeft", "37"),
            (PressKey::ArrowRight, "ArrowRight", "39"),
        ] {
            let js = press_js(key);
            assert!(js.contains(&format!("const key = \"{name}\"")));
            assert!(js.contains(&format!("key === '{name}' ? {code}")));
            assert!(js
                .contains("if (downAllowed && active && active.scrollWidth > active.clientWidth)"));
            assert!(js.contains("const direction = key === 'ArrowLeft' ? -1 : 1"));
            assert!(js.contains("active.scrollBy({ left: direction * 40, behavior: 'instant' })"));
            assert!(js.contains("dispatchKey(target, 'keyup')"));
            assert!(!js.contains("window.present"));
        }
    }

    #[test]
    fn scroll_pumps_headless_viewport_callbacks_after_settling() {
        let settle = SCROLL_JS
            .find("await hwatuSleep(0);")
            .expect("scroll settles");
        let pump = SCROLL_JS
            .find("__hwatuNative?.pumpViewport?.();")
            .expect("scroll pumps");
        assert!(
            pump > settle,
            "viewport pump must run after the scroll settles"
        );
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
        assert!(js.contains("challenge still present"));
        assert!(!js.contains("2captcha"));
        assert!(!js.contains("anti-captcha"));
        assert!(!js.contains("capsolver"));
        let stop = crate::stops::challenge_blocked(&serde_json::json!({
            "status": "manual_required",
            "challenge_type": "turnstile",
            "url": "https://x",
        }));
        assert_eq!(stop["reason"], "challenge");
        assert_eq!(stop["retry"]["command"], "show");
    }

    #[test]
    fn expect_watch_js_uses_native_tick_after_initial_state() {
        let spec = ExpectSpec::new(
            "#status".into(),
            None,
            None,
            Some("ready".into()),
            false,
            true,
            7,
        );
        let initial = expect_watch_js(&spec, true);
        assert!(initial.contains("const force = true"));
        assert!(initial.contains("#status"));
        assert!(initial.contains("ready"));
        assert!(initial.contains("wantVisible = true"));

        let follow_up = expect_watch_js(&spec, false);
        assert!(follow_up.contains("const force = false"));
        assert!(follow_up.contains("window.__hwatuExpectDirty = false"));
        assert!(!follow_up.contains("skipped: true"));
        assert!(follow_up.contains("__hwatuExpectVersion"));
    }

    #[test]
    fn visibility_inspector_scrolls_restores_and_detects_layout_mutation() {
        let js = VISIBILITY_INSPECTOR_JS;
        assert!(js.contains("const original ="));
        assert!(js.contains("scrollIntoView"));
        assert!(js.contains("finally"));
        assert!(js.contains("window.scrollTo(original.x, original.y)"));
        assert!(js.contains("ancestorScroll"));
        assert!(js.contains("ancestor_scroll_restored"));
        assert!(js.contains("scroll_restored"));
        assert!(js.contains("layout_changed"));
        assert!(js.contains("hwatuRectChanged(original.rect, restored.rect)"));
        assert!(js.contains("document_before"));
        assert!(js.contains("document_after"));
        assert!(js.contains("enumerable: false"));
    }

    #[test]
    fn visibility_inspector_accumulates_ancestor_opacity() {
        let js = VISIBILITY_INSPECTOR_JS;
        assert!(js.contains("for (let n = el; n instanceof Element; n = n.parentElement)"));
        assert!(js.contains("effectiveOpacity *= opacity"));
        assert!(js.contains("effective_opacity"));
        assert!(js.contains("opacity_chain"));
    }

    #[test]
    fn expect_watch_visibility_requires_stable_samples_and_checks_occlusion() {
        let spec = ExpectSpec::new("#target".into(), None, None, None, false, true, 1);
        let js = expect_watch_js(&spec, true);
        assert!(js.contains("scrollIntoView"));
        assert!(js.contains("['center'"));
        for corner in ["top-left", "top-right", "bottom-left", "bottom-right"] {
            assert!(js.contains(corner), "missing visibility sample {corner}");
        }
        assert!(js.contains("outside viewport after scroll"));
        assert!(js.contains("point covered by"));
        assert!(js.contains("__hwatuVisibilitySamples"));
        assert!(js.contains("visibility.signature"));
        assert!(js.contains("previousSample.version !== sampleVersion"));
        assert!(js.contains("stabilizing: true"));
    }

    #[test]
    fn expect_event_kind_is_advertised_for_subscriptions() {
        assert!(hwatu_ipc::EVENT_KINDS.contains(&"expect"));
    }

    /// The regression these pin (issue #6): the eval form choice must
    /// never compile a failing candidate in the page. The page-world
    /// probe surfaced every function-body eval as a cross-origin-masked
    /// "Script error." console entry (one per `hwatu resize`, one per
    /// sweep viewport). plan_eval_source decides daemon-side and
    /// returns exactly one source string.
    #[test]
    fn expressions_run_wrapped_in_return() {
        for js in ["document.title", "1+1", "{a: 1}", "await fetch('/x')"] {
            let source = plan_eval_source(js).expect("expression accepted");
            assert_eq!(source, format!("return (\n{js}\n);"), "for {js}");
        }
        // Trailing semicolons are stripped from the expression form.
        assert_eq!(
            plan_eval_source("document.title;").unwrap(),
            "return (\ndocument.title\n);"
        );
    }

    #[test]
    fn bodies_with_return_or_await_run_as_is() {
        for js in [
            "return document.title",
            "const n = 6*7; return n",
            "await hwatuSleep(10); return 1",
        ] {
            assert_eq!(plan_eval_source(js).unwrap(), js, "for {js}");
        }
    }

    #[test]
    fn return_less_bodies_route_through_indirect_eval() {
        let source = plan_eval_source("const n = 6*7; n").unwrap();
        assert!(
            source.starts_with("return (0, eval)("),
            "REPL completion-value semantics: {source}"
        );
        assert!(source.contains("const n = 6*7; n"));
    }

    #[test]
    fn statements_that_probe_as_neither_form_still_run() {
        // `throw` parses as a body statement, not an expression.
        let source = plan_eval_source("throw new Error('boom')").unwrap();
        assert!(source.starts_with("return (0, eval)("));
    }

    #[test]
    fn bad_syntax_is_rejected_daemon_side_with_the_parser_message() {
        // Bodies that must run as-is (they use return/await) are the
        // ones whose parse errors used to be compiled in the page;
        // now they are rejected before touching it. Return-less bad
        // syntax still routes through indirect eval as string data
        // and fails at runtime with a normal SyntaxError reply.
        for js in ["return foo(", "await foo("] {
            let err = plan_eval_source(js).expect_err("syntax error rejected");
            assert!(!err.is_empty(), "parser message expected for {js}");
        }
        assert!(plan_eval_source("foo(")
            .expect("routed through indirect eval")
            .starts_with("return (0, eval)("));
    }

    #[test]
    fn wrapper_escape_attempts_do_not_widen_what_runs() {
        // Escaping the async-wrapper braces makes the candidate fail
        // to parse as one function body; the check must reject it (or
        // route it through indirect eval as a string), never let it
        // parse into extra top-level code.
        let js = "} globalThis.pwned = 1; async function x() {";
        match plan_eval_source(js) {
            // Rejected outright: fine.
            Err(_) => {}
            // Accepted only as data inside an indirect eval string.
            Ok(source) => assert!(source.starts_with("return (0, eval)(")),
        }
    }
}
