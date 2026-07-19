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
use hwatu_ipc::Response;
use std::cell::RefCell;
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

/// Pick the target window: explicit id, else the focused window, else
/// the only window. Ambiguity is an error rather than a guess: an
/// agent driving the wrong window is worse than a retry with an id.
fn resolve(daemon: &Rc<Daemon>, id: Option<u64>) -> Result<Rc<BrowserWindow>, Response> {
    let windows = daemon.windows.borrow();
    if let Some(id) = id {
        return windows
            .get(&id)
            .cloned()
            .ok_or_else(|| Response::err(format!("no window {id}")));
    }
    if let Some(focused) = windows.values().find(|w| w.window.is_active()) {
        return Ok(focused.clone());
    }
    match windows.len() {
        0 => Err(Response::err("no windows open")),
        1 => Ok(windows.values().next().cloned().expect("len checked")),
        n => Err(Response::err(format!(
            "{n} windows open and none focused; pass an explicit id (see `hwatu list`)"
        ))),
    }
}

/// Live WebView of a window, reviving it from a discard first.
fn live_view(win: &Rc<BrowserWindow>) -> Result<webkit6::WebView, Response> {
    win.restore();
    win.live_webview()
        .ok_or_else(|| Response::err("window has no live webview"))
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

/// Run `js` as an async function body in the page. A returned Promise
/// is awaited by WebKit before the callback fires.
pub fn eval(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
    js: String,
    timeout_ms: Option<u64>,
    reply: Reply,
) {
    let reply = OnceReply::new(reply);
    let view = match resolve(daemon, id).and_then(|w| live_view(&w)) {
        Ok(v) => v,
        Err(resp) => return reply.send(resp),
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

    view.call_async_javascript_function(&js, None, None, None, Some(&cancellable), move |result| {
        match result {
            Ok(value) => reply.send(Response::value(jsc_to_json(&value))),
            Err(e) => reply.send(Response::err(format!("eval failed: {e}"))),
        }
    });
}

/// Navigate a window; with `wait`, reply only once the load finishes.
pub fn navigate(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
    url: String,
    wait: bool,
    timeout_ms: Option<u64>,
    reply: Reply,
) {
    let reply = OnceReply::new(reply);
    let win = match resolve(daemon, id) {
        Ok(w) => w,
        Err(resp) => return reply.send(resp),
    };
    let view = match live_view(&win) {
        Ok(v) => v,
        Err(resp) => return reply.send(resp),
    };

    let url = crate::ipc_server::normalize_url(url);
    if !wait {
        view.load_uri(&url);
        return reply.send(Response::window(win.info()));
    }

    arm_timeout(reply.clone(), timeout_ms, "navigate");
    wire_load_finished(&view, {
        let reply = reply.clone();
        move || reply.send(Response::window(win.info()))
    });
    view.load_uri(&url);
}

/// Reply once the window's current load settles.
pub fn wait_load(daemon: &Rc<Daemon>, id: Option<u64>, timeout_ms: Option<u64>, reply: Reply) {
    let reply = OnceReply::new(reply);
    let win = match resolve(daemon, id) {
        Ok(w) => w,
        Err(resp) => return reply.send(resp),
    };
    let view = match live_view(&win) {
        Ok(v) => v,
        Err(resp) => return reply.send(resp),
    };

    if !view.is_loading() {
        return reply.send(Response::window(win.info()));
    }
    arm_timeout(reply.clone(), timeout_ms, "wait_load");
    wire_load_finished(&view, move || reply.send(Response::window(win.info())));
}

/// Call `done` once on the next `LoadEvent::Finished`, then disconnect.
/// Finished fires for both successful and failed loads (WebKit follows
/// load-failed with Finished), so callers always get an answer.
fn wire_load_finished(view: &webkit6::WebView, done: impl FnOnce() + 'static) {
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
            done();
        }
    });
    handler.replace(Some(id));
}

/// Capture the visible viewport as a PNG on disk.
pub fn screenshot(daemon: &Rc<Daemon>, id: Option<u64>, path: Option<String>, reply: Reply) {
    let reply = OnceReply::new(reply);
    let win = match resolve(daemon, id) {
        Ok(w) => w,
        Err(resp) => return reply.send(resp),
    };
    let view = match live_view(&win) {
        Ok(v) => v,
        Err(resp) => return reply.send(resp),
    };

    let target = path.map(std::path::PathBuf::from).unwrap_or_else(|| {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("hwatu-shot-{}-{ts}.png", win.id))
    });

    view.snapshot(
        webkit6::SnapshotRegion::Visible,
        webkit6::SnapshotOptions::NONE,
        gio::Cancellable::NONE,
        move |result| match result {
            Ok(texture) => {
                use gdk::prelude::TextureExt;
                match texture.save_to_png(&target) {
                    Ok(()) => reply.send(Response::path(target.to_string_lossy())),
                    Err(e) => reply.send(Response::err(format!(
                        "screenshot write to {} failed: {e}",
                        target.display()
                    ))),
                }
            }
            Err(e) => reply.send(Response::err(format!("screenshot failed: {e}"))),
        },
    );
}

/// Set a file input's files from a path on disk. The bytes travel
/// into the page as base64 inside a JS snippet, are decoded to a
/// `File`, and assigned through `DataTransfer` — the same technique
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
    use super::base64;

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
}
