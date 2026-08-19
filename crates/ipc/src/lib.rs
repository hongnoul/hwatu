// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! Wire protocol between the `hana` client and the `hwatud` daemon.
//!
//! Newline-delimited JSON over a Unix domain socket or authenticated loopback
//! TCP. A connection may carry multiple [`Request`] lines and receives one
//! [`Response`] line per request, strictly in request order. Legacy one-shot
//! clients still work: connect, send one request, read one response,
//! disconnect. A [`Request::Subscribe`] hands the connection to the event
//! stream and does not accept further request lines.

use serde::{Deserialize, Serialize};
use std::io::{self, BufRead};
use std::path::PathBuf;

/// Resolve the daemon socket path: `$XDG_RUNTIME_DIR/hwatu.sock`,
/// falling back to `/tmp/hwatu-$UID.sock`.
#[cfg(unix)]
pub fn socket_path() -> PathBuf {
    if let Ok(path) = std::env::var("HWATU_SOCKET") {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(dir).join("hwatu.sock");
    }
    let uid = unsafe { libc_geteuid() };
    PathBuf::from(format!("/tmp/hwatu-{uid}.sock"))
}

// Tiny FFI shim so the client stays dependency-free.
#[cfg(unix)]
extern "C" {
    #[link_name = "geteuid"]
    fn libc_geteuid() -> u32;
}

/// Default port for hwatu's optional loopback TCP transport.
pub const HWATU_TCP_PORT: u16 = 8741;

/// Maximum serialized request or response frame, including its newline.
pub const MAX_FRAME_BYTES: usize = 32 * 1024 * 1024;

/// Maximum authentication frame. Tokens are intentionally tiny compared with
/// normal protocol frames so unauthenticated peers cannot reserve large buffers.
pub const MAX_AUTH_FRAME_BYTES: usize = 4 * 1024;

/// Authentication token bounds. The minimum rejects guessable passwords; the
/// maximum keeps the complete authentication frame comfortably bounded.
pub const MIN_TOKEN_BYTES: usize = 32;
pub const MAX_TOKEN_BYTES: usize = 1024;

/// Maximum decoded inline payload. Base64 expansion keeps one payload below
/// [`MAX_FRAME_BYTES`] with more than 10 MiB left for JSON and metadata.
pub const INLINE_MAX_BYTES: usize = 16 * 1024 * 1024;

pub fn validate_token(token: &str) -> Result<(), String> {
    if token.len() < MIN_TOKEN_BYTES {
        return Err(format!(
            "authentication token must be at least {MIN_TOKEN_BYTES} bytes"
        ));
    }
    if token.len() > MAX_TOKEN_BYTES {
        return Err(format!(
            "authentication token must not exceed {MAX_TOKEN_BYTES} bytes"
        ));
    }
    Ok(())
}

/// Load a bearer token from a small private regular file. One conventional
/// trailing CR/LF sequence is ignored. Unix group/other permissions are
/// rejected so a copied token cannot silently become machine-readable.
pub fn load_token_file(path: impl AsRef<std::path::Path>) -> Result<String, String> {
    let path = path.as_ref();
    let metadata = std::fs::metadata(path)
        .map_err(|error| format!("cannot inspect token file {}: {error}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!(
            "token path {} is not a regular file",
            path.display()
        ));
    }
    if metadata.len() > (MAX_TOKEN_BYTES + 2) as u64 {
        return Err(format!("token file {} is too large", path.display()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(format!(
                "token file {} must not be accessible by group or other users",
                path.display()
            ));
        }
    }
    let token = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read token file {}: {error}", path.display()))?
        .trim_end_matches(['\r', '\n'])
        .to_string();
    validate_token(&token)?;
    Ok(token)
}

/// Serialize one newline-delimited JSON frame while enforcing its wire limit.
pub fn encode_frame<T: Serialize>(value: &T, max_bytes: usize) -> Result<Vec<u8>, String> {
    let mut frame = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    if frame
        .len()
        .checked_add(1)
        .is_none_or(|length| length > max_bytes)
    {
        return Err(format!(
            "frame is {} bytes, exceeding the {max_bytes}-byte limit",
            frame.len().saturating_add(1)
        ));
    }
    frame.push(b'\n');
    Ok(frame)
}

/// Read one newline-delimited frame without allowing `read_line` to allocate
/// beyond the protocol limit. The returned bytes exclude the newline.
pub fn read_frame<R: BufRead>(reader: &mut R, max_bytes: usize) -> io::Result<Option<Vec<u8>>> {
    let mut frame = Vec::with_capacity(max_bytes.min(8 * 1024));
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if frame.is_empty() {
                Ok(None)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "connection closed in the middle of a frame",
                ))
            };
        }

        if let Some(newline) = available.iter().position(|byte| *byte == b'\n') {
            let consumed = newline + 1;
            if frame.len().saturating_add(consumed) > max_bytes {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("frame exceeds the {max_bytes}-byte limit"),
                ));
            }
            frame.extend_from_slice(&available[..newline]);
            reader.consume(consumed);
            return Ok(Some(frame));
        }

        if frame.len().saturating_add(available.len()) >= max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("frame exceeds the {max_bytes}-byte limit"),
            ));
        }
        let consumed = available.len();
        frame.extend_from_slice(available);
        reader.consume(consumed);
    }
}

/// Where a client reaches the daemon. TCP authorities remain unresolved until
/// connect time so clients can try every address returned for a hostname.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    Unix(PathBuf),
    Tcp(String),
}

/// Resolve the configured endpoint. `HWATU_ENDPOINT` takes precedence over
/// the legacy Unix-only `HWATU_SOCKET` setting.
pub fn endpoint() -> Result<Endpoint, String> {
    if let Ok(value) = std::env::var("HWATU_ENDPOINT") {
        if !value.trim().is_empty() {
            return parse_endpoint(value.trim());
        }
    }
    #[cfg(unix)]
    {
        Ok(Endpoint::Unix(socket_path()))
    }
    #[cfg(not(unix))]
    {
        Err("HWATU_ENDPOINT must be set on this platform".to_string())
    }
}

/// Parse one endpoint without touching DNS or process environment.
pub fn parse_endpoint(value: &str) -> Result<Endpoint, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("endpoint cannot be empty".to_string());
    }
    if let Some(authority) = value.strip_prefix("tcp://") {
        validate_tcp_authority(authority)?;
        return Ok(Endpoint::Tcp(authority.to_string()));
    }
    if let Some(path) = value.strip_prefix("unix://") {
        if path.is_empty() {
            return Err("unix endpoint path cannot be empty".to_string());
        }
        return Ok(Endpoint::Unix(PathBuf::from(path)));
    }
    if value.contains("://") {
        return Err(format!("unsupported endpoint scheme in {value:?}"));
    }
    if value.contains(':') {
        validate_tcp_authority(value)?;
        return Ok(Endpoint::Tcp(value.to_string()));
    }
    Ok(Endpoint::Unix(PathBuf::from(value)))
}

fn validate_tcp_authority(authority: &str) -> Result<(), String> {
    if authority.chars().any(char::is_whitespace) {
        return Err(format!(
            "whitespace is not allowed in TCP authority {authority:?}"
        ));
    }
    let (_host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let (host, port) = rest
            .split_once("]:")
            .ok_or_else(|| format!("invalid bracketed TCP authority {authority:?}"))?;
        if host.is_empty() || host.contains('[') || port.contains(':') {
            return Err(format!("invalid TCP authority {authority:?}"));
        }
        (host, port)
    } else {
        let (host, port) = authority
            .rsplit_once(':')
            .ok_or_else(|| format!("TCP endpoint needs host:port, got {authority:?}"))?;
        if host.is_empty() || host.contains(':') || host.contains('/') {
            return Err(format!("invalid TCP host in {authority:?}"));
        }
        (host, port)
    };
    if port.parse::<u16>().ok().filter(|port| *port != 0).is_none() {
        return Err(format!("invalid TCP port in {authority:?}"));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    /// Open a new browser window. The daemon adopts a prewarmed WebView,
    /// so this returns as soon as the window is mapped.
    Open {
        url: Option<String>,
        /// Wayland app_id / X11 WM_CLASS for tiling-WM window rules.
        #[serde(default)]
        app_id: Option<String>,
        /// How the window is shown; see [`OpenMode`]. Absent on the
        /// wire means [`OpenMode::Normal`], so old clients keep working.
        #[serde(default)]
        mode: OpenMode,
        /// Cookie/site-data isolation (platform item 6): windows with
        /// the same profile name share a session; different profiles
        /// never share cookies, storage, or logins. Absent = the
        /// daemon's default (persistent) session. The CLI sends
        /// `HWATU_PROFILE` when set (`auto` derives a per-worktree
        /// name, so N agents in N worktrees isolate with zero flags).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        profile: Option<String>,
    },
    /// List open windows.
    List,
    /// Close a window by id.
    Close { id: u64 },
    /// Control the built-in content blocker.
    Adblock { action: AdblockCmd },
    /// Ask the daemon to exit.
    Quit,
    /// Health check / used by the client to detect a live daemon.
    Ping,

    // ---- automation (agent integration) ----------------------------
    // These primitives let coding agents (e.g. jcode) drive a window:
    // run JS in the page, navigate, screenshot, and wait for loads.
    // `id: None` targets the focused window, else the only window.
    /// Run JavaScript in a window's page. `js` is a *function body*
    /// (so `return` works), and a returned Promise is awaited. The
    /// result comes back as JSON in [`Response::Ok::value`].
    Eval {
        #[serde(default)]
        id: Option<u64>,
        js: String,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Navigate an existing window. `wait` (default true) blocks the
    /// response until the load reaches `until` (default: settled) or
    /// `timeout_ms` expires.
    Navigate {
        #[serde(default)]
        id: Option<u64>,
        url: String,
        #[serde(default = "default_true")]
        wait: bool,
        /// How far the load must progress before the reply (see
        /// [`LoadStage`]). Absent on the wire means `Settled`, so old
        /// clients keep the full-load semantics they were built for.
        #[serde(default)]
        until: LoadStage,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Capture the page as a PNG. Writes to `path` (or a temp file)
    /// and returns the file path in [`Response::Ok::path`].
    Screenshot {
        #[serde(default)]
        id: Option<u64>,
        #[serde(default)]
        path: Option<String>,
        /// Capture the entire document instead of just the visible
        /// viewport. Agents hunting for below-the-fold content should
        /// use this instead of scroll-and-shoot loops.
        #[serde(default)]
        full: bool,
        /// Return PNG bytes in [`Response::Ok::data`] instead of writing a
        /// daemon-host path.
        #[serde(default, skip_serializing_if = "is_false")]
        data: bool,
    },
    /// Block until the window's load reaches `until` (default:
    /// settled) or `timeout_ms` expires.
    WaitLoad {
        #[serde(default)]
        id: Option<u64>,
        #[serde(default)]
        until: LoadStage,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// One-roundtrip verification pass: open a headless window, load
    /// `url`, wait for `until`, optionally run JS and take a
    /// screenshot, then close the window (unless `keep`). Replies with
    /// everything at once in [`Response::Ok::value`]: final url,
    /// title, eval result, screenshot path, console entries, timings.
    /// Collapses the open/wait/eval/shot/close agent loop (5 process
    /// spawns + 5 socket roundtrips) into one.
    Check {
        /// URL to load. Exactly one of `url` and `render` must be
        /// given. Optional (with a default) so old clients that always
        /// sent it keep working, and new render-only requests can omit
        /// it; an old daemon rejects a render-only request with a
        /// clean "missing field `url`" error.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        /// Inline HTML to load directly (`webkit_web_view_load_html`)
        /// instead of navigating to a URL. An agent with generated
        /// markup in hand needs no temp file and no HTTP server.
        /// Capped at [`RENDER_MAX_BYTES`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        render: Option<String>,
        /// Base URL for resolving relative references (images, CSS,
        /// scripts) in rendered markup; only with `render`. Without
        /// it the document loads as `about:blank`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base: Option<String>,
        /// JS to run once the load reaches `until` (expression or
        /// function body, same semantics as [`Request::Eval`]).
        #[serde(default)]
        eval: Option<String>,
        /// Take a screenshot (to a temp file unless `shot_path`).
        #[serde(default)]
        shot: bool,
        /// Screenshot destination; implies `shot`.
        #[serde(default)]
        shot_path: Option<String>,
        /// Return screenshot bytes inline for a remote client.
        #[serde(default, skip_serializing_if = "is_false")]
        shot_data: bool,
        /// Screenshot the full document instead of the viewport.
        #[serde(default)]
        full: bool,
        /// Pixel-diff the loaded page against this baseline PNG and
        /// include the score/regions in the reply (same output as
        /// [`Request::Diff`]). Folds the verify loop's pixel tier into
        /// the one roundtrip: `--eval` answers "is the DOM right",
        /// `--baseline` answers "does it look right".
        #[serde(default)]
        baseline: Option<String>,
        /// Base64 PNG supplied by a client that cannot share daemon paths.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        baseline_data: Option<String>,
        /// Per-channel diff tolerance 0-255 (default 8); only with `baseline`.
        #[serde(default)]
        tolerance: Option<u8>,
        /// Write a mismatch heatmap PNG here; only with `baseline`.
        #[serde(default)]
        heatmap: Option<String>,
        /// Return the generated heatmap inline instead of writing a path.
        #[serde(default, skip_serializing_if = "is_false")]
        heatmap_data: bool,
        /// Load stage gating eval/shot (default: settled).
        #[serde(default)]
        until: LoadStage,
        /// Keep the window open and report its id instead of closing.
        #[serde(default)]
        keep: bool,
        #[serde(default)]
        timeout_ms: Option<u64>,
        /// Multi-viewport sweep: run the same pass (eval, shot,
        /// baseline diff) at each of these sizes sequentially on one
        /// pooled window, and reply with per-viewport results under
        /// `viewports: [{size, load_ms, eval, shot, diff, ...}]`.
        /// Empty/absent means the classic single-viewport check with
        /// the identical reply shape as before. Screenshot paths get
        /// a `-<WxH>` suffix per size; `baseline_dir` supplies a
        /// per-size baseline `<dir>/<WxH>.png`.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        viewports: Vec<Viewport>,
        /// Directory of per-size baseline PNGs (`<dir>/360x640.png`)
        /// for the viewport sweep; each size is diffed against its own
        /// file. Only with `viewports`. Mutually exclusive with
        /// `baseline` (which is a single file for a single size).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        baseline_dir: Option<String>,
    },
    /// Speculatively start loading `url` in a parked headless window,
    /// replying immediately. The next [`Request::Check`] of the same
    /// URL adopts the window instead of navigating a fresh one, so the
    /// load happens while the agent is still thinking (or while the
    /// dev server rebuilds) and the check pays ~0 load latency.
    /// Unclaimed prefetches expire after a short TTL. Fire-and-forget:
    /// a prefetch is never required for correctness.
    Prefetch { url: String },
    /// Detect CAPTCHA / anti-bot challenge UI, and optionally wait for a
    /// human to clear it. This does not solve or bypass the challenge; it
    /// returns structured state so an agent can pause/resume safely.
    Challenge {
        #[serde(default)]
        id: Option<u64>,
        /// When true, poll until the challenge disappears or timeout fires.
        #[serde(default)]
        wait: bool,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Present (raise/focus) a window.
    Focus { id: u64 },
    /// The inverse of [`Request::Focus`]: unmap the window and return
    /// it to the mode it had before a `Focus` promoted it (headless
    /// for windows that were never promoted). Lets agents hand a
    /// window back out of the user's way once it no longer needs
    /// human attention.
    Unfocus { id: u64 },
    /// Select a file through WebKit's native chooser. Local clients pass a
    /// daemon-visible path. Remote clients pass base64 data that the daemon
    /// stages privately before activating the same chooser path.
    Upload {
        #[serde(default)]
        id: Option<u64>,
        selector: String,
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        data: Option<String>,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Scroll the page and report where it landed. Exactly one way to
    /// say "where": `selector` (scrolled into view, disambiguated by
    /// `nth`/`contains`), `to_y` (absolute pixels), or `by_pages`
    /// (relative viewport-heights; default 1.0 when nothing is given).
    /// The response `value` reports match count, the matched element's
    /// text, and the resulting scroll position, so an agent always
    /// knows what it hit and whether the bottom was reached.
    Scroll {
        #[serde(default)]
        id: Option<u64>,
        /// CSS selector to scroll into view (centered).
        #[serde(default)]
        selector: Option<String>,
        /// 0-based index among selector matches (default 0).
        #[serde(default)]
        nth: Option<u32>,
        /// Keep only selector matches whose text contains this.
        #[serde(default)]
        contains: Option<String>,
        /// Absolute scroll target in CSS pixels.
        #[serde(default)]
        to_y: Option<f64>,
        /// Relative scroll in viewport heights (negative = up).
        #[serde(default)]
        by_pages: Option<f64>,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Token-cheap structured page state: url, title, visible text
    /// (bounded), and an indexed list of interactable elements. The
    /// indices ("refs") are remembered by the page, so a follow-up
    /// [`Request::Click`]/[`Request::Type`] can target `ref: n`
    /// without a selector. The cheap alternative to a screenshot.
    Snapshot {
        #[serde(default)]
        id: Option<u64>,
        /// Return only what changed since the last `--diff` snapshot
        /// of this window (`{added, removed, changed, unchanged_count}`)
        /// instead of the full page state. The first diff snapshot of
        /// a window (and the first after a navigation) returns the
        /// full snapshot with `"baseline_established": true`. Absent
        /// on the wire means `false`, so old clients keep the full
        /// snapshot they were built for; an old daemon ignores the
        /// field and answers a new client with a full snapshot.
        #[serde(default, skip_serializing_if = "is_false")]
        diff: bool,
        /// Include each interactable's viewport CSS rectangle as
        /// `[x, y, width, height]`. Disabled by default to keep snapshots
        /// token-cheap and preserve the original response shape.
        #[serde(default, skip_serializing_if = "is_false")]
        rect: bool,
        /// Character budget for the reply (verification P3): the
        /// snapshot degrades coarse-to-fine instead of truncating
        /// arbitrarily — page text shrinks first, then interactable
        /// text/href fields shorten, then interactables reduce to
        /// landmark counts. 0/absent = unbudgeted (classic shape).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        budget: Option<usize>,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Click an element: by CSS `selector` (disambiguated by
    /// `nth`/`contains`, like Scroll) or by a `ref` from the last
    /// [`Request::Snapshot`]. Dispatches real pointer/mouse events;
    /// the response reports what was hit (match count, tag, text).
    /// With `trusted`, the daemon must use compositor/toolkit input
    /// synthesis so page handlers see `event.isTrusted === true`.
    Click {
        #[serde(default)]
        id: Option<u64>,
        #[serde(default)]
        selector: Option<String>,
        #[serde(default)]
        nth: Option<u32>,
        #[serde(default)]
        contains: Option<String>,
        /// Interactable index from the last snapshot of this window.
        #[serde(default)]
        r#ref: Option<u32>,
        /// Use trusted native input instead of JS-dispatched events.
        #[serde(default, skip_serializing_if = "is_false")]
        trusted: bool,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Type text into an input/textarea/select/contenteditable,
    /// targeted like [`Request::Click`]. Values are set through the
    /// native setter and followed by `input`/`change` events, so
    /// framework-controlled inputs (React et al.) see the change.
    Type {
        #[serde(default)]
        id: Option<u64>,
        #[serde(default)]
        selector: Option<String>,
        #[serde(default)]
        nth: Option<u32>,
        #[serde(default)]
        contains: Option<String>,
        /// Interactable index from the last snapshot of this window.
        #[serde(default)]
        r#ref: Option<u32>,
        text: String,
        /// Use trusted native input instead of JS-dispatched events.
        #[serde(default, skip_serializing_if = "is_false")]
        trusted: bool,
        /// Replace the current value (default) instead of appending.
        #[serde(default = "default_true")]
        clear: bool,
        /// Press Enter afterwards (submits the enclosing form if the
        /// page did not handle the keydown itself).
        #[serde(default)]
        enter: bool,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Press a page-local keyboard key without presenting or focusing the
    /// native window. `Tab` advances DOM focus, `Enter` activates the focused
    /// element, and `Escape` runs dialog cancel semantics, including in
    /// headless windows.
    Press {
        #[serde(default)]
        id: Option<u64>,
        key: PressKey,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Paste from the compositor clipboard into an element targeted like
    /// [`Request::Click`]. Always uses trusted native input synthesis: the
    /// daemon clicks/focuses the target, then asks `wtype` to press Ctrl+V
    /// while the trusted input session owns compositor focus.
    Paste {
        #[serde(default)]
        id: Option<u64>,
        #[serde(default)]
        selector: Option<String>,
        #[serde(default)]
        nth: Option<u32>,
        #[serde(default)]
        contains: Option<String>,
        /// Interactable index from the last snapshot of this window.
        #[serde(default)]
        r#ref: Option<u32>,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Read the window's console/error/network capture buffer:
    /// console.* calls, uncaught exceptions, unhandled rejections,
    /// failed resource loads, and HTTP >= 400 responses. `clear`
    /// drains what was read, so a verify loop can diff runs.
    Console {
        #[serde(default)]
        id: Option<u64>,
        #[serde(default)]
        clear: bool,
        /// Return at most the last N entries.
        #[serde(default)]
        limit: Option<usize>,
    },
    /// Read the window's structured network request log: every
    /// resource load (method, final url, HTTP status, inferred type,
    /// start offset / duration in ms), success and failure alike,
    /// captured from WebKit's resource-load signals into a bounded
    /// per-window ring buffer. `clear` drains what was read, so a
    /// verify loop can diff runs ("did the POST to /api/charge return
    /// 200"). Observation only: WebKitGTK exposes no route
    /// interception.
    Net {
        #[serde(default)]
        id: Option<u64>,
        #[serde(default)]
        clear: bool,
        /// Return at most the last N entries.
        #[serde(default)]
        limit: Option<usize>,
    },
    /// Extract the page's motion spec: every CSS animation, transition
    /// and Web-Animations-API animation as structured JSON (keyframes,
    /// duration, delay, easing, iteration count), plus `@keyframes`
    /// rules and declared-but-idle `transition-*` styles from CSSOM.
    /// Motion is numbers, not pixels: an agent copies/verifies easing
    /// curves exactly instead of eyeballing frames.
    Motion {
        #[serde(default)]
        id: Option<u64>,
        /// Observe the live page instead of only reading declared
        /// animation: sample moving elements per frame for
        /// `observe_ms`, then fit motion models (linear velocity,
        /// loop period, easing curve) in the daemon. Catches
        /// script-driven motion (requestAnimationFrame marquees,
        /// JS tickers) that `getAnimations()` cannot see.
        #[serde(default)]
        observe: bool,
        /// Observation window in ms (default 2500).
        #[serde(default)]
        observe_ms: Option<u64>,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Freeze animation time: pause every animation on the page and
    /// set its currentTime, so the page renders a deterministic frame
    /// for screenshot/diff. `time_ms` seeks to an absolute time;
    /// `progress` (0.0–1.0) seeks each animation proportionally to its
    /// own duration. `resume` unpauses everything instead.
    Seek {
        #[serde(default)]
        id: Option<u64>,
        /// Absolute animation time in ms (applied to every animation).
        #[serde(default)]
        time_ms: Option<f64>,
        /// Per-animation fractional progress (0.0 = start, 1.0 = end).
        #[serde(default)]
        progress: Option<f64>,
        /// Unpause all animations, restoring live playback.
        #[serde(default)]
        resume: bool,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Perceptual pixel diff of two windows (or a window against a
    /// baseline PNG). Returns a match score (percent of pixels within
    /// tolerance), the bounding boxes of the largest mismatched
    /// regions, and optionally writes a heatmap PNG highlighting the
    /// differences. This is the feedback signal that lets an agent
    /// *converge* on pixel-perfect instead of eyeballing screenshots.
    Diff {
        /// First window.
        id: u64,
        /// Second window to compare against...
        #[serde(default)]
        other: Option<u64>,
        /// ...or a baseline PNG on disk (exactly one of the two).
        #[serde(default)]
        baseline: Option<String>,
        /// Base64 PNG supplied by a remote client.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        baseline_data: Option<String>,
        /// Per-channel tolerance 0-255 before a pixel counts as
        /// different (default 8, forgiving of AA/compression noise).
        #[serde(default)]
        tolerance: Option<u8>,
        /// Write a heatmap PNG (mismatches in red over a dimmed base)
        /// to this path.
        #[serde(default)]
        heatmap: Option<String>,
        /// Return the generated heatmap inline instead of writing a path.
        #[serde(default, skip_serializing_if = "is_false")]
        heatmap_data: bool,
        /// Diff the full document instead of the visible viewport.
        #[serde(default)]
        full: bool,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Assert page state. Without `watch`, polls until it holds or
    /// `timeout_ms` expires: `selector` matches an element
    /// (disambiguated by `nth`/`contains`), optionally with text
    /// containing `text`; `absent` inverts (assert no match). With
    /// `watch`, installs a resident monitor that emits `expect` events on
    /// initial state, every truth-value flip, and terminal navigation.
    Expect {
        #[serde(default)]
        id: Option<u64>,
        selector: String,
        #[serde(default)]
        nth: Option<u32>,
        /// Keep only matches whose text contains this (a filter, like
        /// Click's).
        #[serde(default)]
        contains: Option<String>,
        /// Require the matched element's text to contain this. Unlike
        /// `contains`, a `text` mismatch fails the assertion and the
        /// error reports the element's actual text.
        #[serde(default)]
        text: Option<String>,
        /// Assert the selector matches nothing instead.
        #[serde(default)]
        absent: bool,
        /// Require the matched element to be actually visible: nonzero
        /// box, not display:none/visibility:hidden/opacity:0, scrolled
        /// into view when fully off-screen, and not covered at its center
        /// or four corners (elementFromPoint resolves inside it). Catches
        /// rendered-but-invisible UI that a bare existence check false-passes.
        #[serde(default)]
        visible: bool,
        /// Poll deadline (default 5000 ms; 0 = a single check).
        #[serde(default)]
        timeout_ms: Option<u64>,
        /// Install a resident assertion watcher instead of replying only
        /// when the assertion first holds/fails.
        #[serde(default)]
        watch: bool,
    },
    /// Control a page's *virtual clock*. Where [`Request::Seek`] pins
    /// declarative animations (CSS/WAAPI), Clock also freezes the
    /// clocks script-driven animation reads: `requestAnimationFrame`,
    /// `performance.now`, `Date.now`, zero-argument `Date` construction,
    /// and `setTimeout`/`setInterval` are wrapped at document start behind
    /// one controllable timeline, and `document.getAnimations()` is driven
    /// from the same timeline. A
    /// rAF-driven marquee that Seek cannot touch freezes under
    /// `pause` and advances deterministically under `step`.
    Clock {
        #[serde(default)]
        id: Option<u64>,
        action: ClockAction,
        /// Milliseconds: the amount for `step`, the absolute virtual
        /// time for `set`. Ignored by `pause`/`resume`.
        #[serde(default)]
        ms: Option<f64>,
        /// PRNG seed for `seed`. Ignored by every other action.
        #[serde(default)]
        seed: Option<u64>,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Set a window's viewport size (CSS pixels). For headless windows
    /// this re-allocates the offscreen toplevel; for mapped windows it
    /// resizes the window (compositor policy permitting). The point is
    /// matrix verification: responsive pages must be checked at several
    /// widths, and a resize on a warm window costs milliseconds while a
    /// fresh browser context costs seconds.
    Resize {
        #[serde(default)]
        id: Option<u64>,
        width: i32,
        height: i32,
    },
    /// Execute a bounded sequence of allowlisted automation actions in one
    /// request. The daemon validates the whole batch before touching the
    /// page, then runs actions in order and stops on the first failed step.
    /// The reply is a [`BatchResult`] serialized in [`Response::Ok::value`]
    /// under `batch`, with skipped entries for actions that were never run.
    Batch { actions: Vec<Request> },
    /// Hold the connection open and stream server-initiated [`Event`]s
    /// as JSON lines (the push half of the protocol; everything else
    /// stays one-shot). The daemon answers with one `subscribed`
    /// event, then pushes matching events until the client closes the
    /// connection. No daemon-side queues for dead clients: a dropped
    /// or stuck connection (write buffer over its cap) is discarded,
    /// never buffered for later.
    Subscribe {
        /// Only these event kinds (e.g. `["load", "console"]`).
        /// Absent = all kinds.
        #[serde(default)]
        kinds: Option<Vec<String>>,
        /// Only events for this window. Absent = all windows.
        #[serde(default)]
        window: Option<u64>,
    },
    /// Query (or clear) the global visit history (roadmap H9). With a
    /// query, returns frecency-ranked completions; empty query returns
    /// the most relevant recent pages. `clear` wipes history and
    /// reports the removed row count.
    History {
        #[serde(default)]
        query: String,
        #[serde(default)]
        limit: Option<usize>,
        #[serde(default)]
        clear: bool,
    },
    /// Clear stored site data (roadmap H16): cookies, local/session
    /// storage, IndexedDB, caches. With `host`, only entries whose
    /// registrable domain matches; without, everything. Also drops
    /// matching per-site decisions (permissions, zoom, dark mode) and,
    /// on a full clear, visit history.
    ClearSiteData {
        #[serde(default)]
        host: Option<String>,
    },
    /// Generalized human hand-off (platform roadmap items 10-11).
    /// Flags "this window needs a person" with a reason. Default is
    /// queueing: the entry waits in the hand-off queue until a human
    /// drains it (respects flow — an agent that needs a human at
    /// 14:02 should not steal focus at 14:02). With `now`, the window
    /// materializes immediately with the reason in the bar (the old
    /// challenge-style behavior, for when the agent is blocked).
    Handoff {
        id: u64,
        reason: String,
        /// Materialize immediately instead of queueing.
        #[serde(default)]
        now: bool,
    },
    /// List pending hand-offs (id, reason, queued_at) or resolve one:
    /// `take` promotes that window to focus and removes the entry,
    /// stamping answered_at so waiting cost is a measured number.
    Handoffs {
        /// Window id to take (present + remove). Absent = list.
        #[serde(default)]
        take: Option<u64>,
    },
    /// Fuzzy jump (roadmap H29): match `query` against open windows
    /// (url + title) first, then visit history. Focuses the best
    /// window match; with `open` (default true), a history-only match
    /// opens a new window on it. Replies with what it did. The
    /// "Spotlight for the web" verb — bind `hwatu jump` in the WM.
    Jump {
        query: String,
        /// Open a window for history-only matches (default true).
        #[serde(default = "default_true")]
        open: bool,
    },
}

/// Maximum number of actions in one [`Request::Batch`]. This bounds daemon
/// memory, validation time, and how long one IPC request can occupy the GTK
/// main loop before giving the caller a progress boundary.
pub const BATCH_MAX_ACTIONS: usize = 32;

impl Request {
    /// Stable human-readable variant name for diagnostics and batch results.
    pub fn kind(&self) -> &'static str {
        match self {
            Request::Open { .. } => "open",
            Request::List => "list",
            Request::Close { .. } => "close",
            Request::Adblock { .. } => "adblock",
            Request::Quit => "quit",
            Request::Ping => "ping",
            Request::Eval { .. } => "eval",
            Request::Navigate { .. } => "navigate",
            Request::Screenshot { .. } => "screenshot",
            Request::WaitLoad { .. } => "wait_load",
            Request::Check { .. } => "check",
            Request::Prefetch { .. } => "prefetch",
            Request::Challenge { .. } => "challenge",
            Request::Focus { .. } => "focus",
            Request::Unfocus { .. } => "unfocus",
            Request::Upload { .. } => "upload",
            Request::Scroll { .. } => "scroll",
            Request::Snapshot { .. } => "snapshot",
            Request::Click { .. } => "click",
            Request::Type { .. } => "type",
            Request::Press { .. } => "press",
            Request::Paste { .. } => "paste",
            Request::Console { .. } => "console",
            Request::Net { .. } => "net",
            Request::Motion { .. } => "motion",
            Request::Seek { .. } => "seek",
            Request::Clock { .. } => "clock",
            Request::Diff { .. } => "diff",
            Request::Expect { .. } => "expect",
            Request::Resize { .. } => "resize",
            Request::Batch { .. } => "batch",
            Request::Subscribe { .. } => "subscribe",
            Request::History { .. } => "history",
            Request::ClearSiteData { .. } => "clear_site_data",
            Request::Handoff { .. } => "handoff",
            Request::Handoffs { .. } => "handoffs",
            Request::Jump { .. } => "jump",
        }
    }

    /// True when this request asks the daemon to execute page JavaScript.
    /// Operators can disable this entire surface at the daemon boundary so
    /// direct CLI, MCP, and raw socket clients all get the same policy.
    pub fn uses_eval(&self) -> bool {
        match self {
            Request::Eval { .. } => true,
            Request::Check { eval, .. } => eval.as_ref().is_some_and(|js| !js.is_empty()),
            Request::Batch { actions } => actions.iter().any(Self::uses_eval),
            _ => false,
        }
    }

    /// Whether this request is safe and coherent as one step inside a batch.
    /// Keep this deliberately smaller than the full protocol: lifecycle,
    /// streaming, browser-global, file-upload, and long-lived watcher actions
    /// stay top-level one-shots until they have dedicated semantics.
    pub fn is_batch_action(&self) -> bool {
        matches!(
            self,
            Request::Eval { .. }
                | Request::Navigate { .. }
                | Request::Screenshot { .. }
                | Request::WaitLoad { .. }
                | Request::Scroll { .. }
                | Request::Snapshot { .. }
                | Request::Click { .. }
                | Request::Type { .. }
                | Request::Press { .. }
                | Request::Paste { .. }
                | Request::Console { .. }
        ) || matches!(self, Request::Expect { watch: false, .. })
    }

    /// Validate a batch before any action executes. The daemon calls this
    /// again even if clients preflight locally: clients are not trusted.
    pub fn validate_batch(actions: &[Request]) -> Result<(), String> {
        if actions.is_empty() {
            return Err("batch must contain at least one action".into());
        }
        if actions.len() > BATCH_MAX_ACTIONS {
            return Err(format!(
                "batch has {} actions; the cap is {}",
                actions.len(),
                BATCH_MAX_ACTIONS
            ));
        }
        for (index, action) in actions.iter().enumerate() {
            if matches!(action, Request::Batch { .. }) {
                return Err(format!(
                    "batch action {index} is nested batch; nested batches are unsupported"
                ));
            }
            if matches!(action, Request::Expect { watch: true, .. }) {
                return Err(format!(
                    "batch action {index} is expect --watch; resident watchers are unsupported in batches"
                ));
            }
            if !action.is_batch_action() {
                return Err(format!(
                    "batch action {index} ({}) is unsupported; allowed actions are eval, navigate, screenshot, wait_load, scroll, snapshot, click, type, press, paste, console, expect",
                    action.kind()
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchStepStatus {
    Ok,
    Error,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchStepResult {
    pub index: usize,
    pub action: String,
    pub status: BatchStepStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<Response>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchResult {
    /// True only when every step ran and succeeded.
    pub complete: bool,
    /// Count of steps actually executed, including a failing step.
    pub executed: usize,
    /// Index of the first failing step, if execution stopped early.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_at: Option<usize>,
    pub steps: Vec<BatchStepResult>,
}

fn default_true() -> bool {
    true
}

/// Cap on inline `render` markup, shared by client and daemon. The
/// protocol is one line-delimited JSON request per connection, so a
/// pathological document would be buffered whole in the daemon's
/// line reader; 8 MiB comfortably covers generated documents (a 1 MB
/// page is already unusually large) while bounding that buffer. The
/// client checks before sending for a fast, clear error; the daemon
/// checks again because clients are not trusted.
pub const RENDER_MAX_BYTES: usize = 8 * 1024 * 1024;

/// One viewport size (CSS pixels) for a multi-viewport check sweep.
/// Serialized as `{ "w": 360, "h": 640 }`; the user-facing form is
/// `360x640` (see [`Viewport::parse`] / [`Viewport::label`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Viewport {
    pub w: i32,
    pub h: i32,
}

impl Viewport {
    /// Parse a `<width>x<height>` size (e.g. `360x640`). Bounds match
    /// the daemon's resize limits so a bad size fails at parse time
    /// on the client instead of mid-sweep on the daemon.
    pub fn parse(value: &str) -> Option<Self> {
        let (w, h) = value.trim().split_once(['x', 'X'])?;
        let (w, h) = (w.trim().parse().ok()?, h.trim().parse().ok()?);
        if !(1..=16384).contains(&w) || !(1..=16384).contains(&h) {
            return None;
        }
        Some(Self { w, h })
    }

    /// Parse a comma-separated size list (`360x640,768x1024`). Any
    /// invalid entry fails the whole list, naming the bad entry.
    pub fn parse_list(value: &str) -> Result<Vec<Self>, String> {
        value
            .split(',')
            .filter(|s| !s.trim().is_empty())
            .map(|s| {
                Self::parse(s).ok_or_else(|| {
                    format!(
                        "bad viewport {:?} (expected <width>x<height>, e.g. 360x640)",
                        s.trim()
                    )
                })
            })
            .collect()
    }

    /// The canonical `<width>x<height>` label used for reply `size`
    /// fields, per-size screenshot suffixes, and baseline filenames.
    pub fn label(&self) -> String {
        format!("{}x{}", self.w, self.h)
    }
}

/// How far a load must progress before a wait releases. Real pages
/// keep loading subresources (fonts, third-party JS, images) long
/// after the DOM is usable; most agent checks only need the DOM, so
/// waiting for the full settle is paying tail latency for nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadStage {
    /// The new document has replaced the old one (WebKit Committed).
    /// Earliest point where evals target the new page; its DOM may
    /// still be streaming in.
    Committed,
    /// `DOMContentLoaded`: the DOM is fully parsed and queryable.
    /// The right default for snapshot/eval checks on real pages.
    Dom,
    /// Full load finished and no follow-up navigation is pending
    /// (every subresource done). The strongest guarantee, and the
    /// wire default for backward compatibility.
    #[default]
    Settled,
}

impl LoadStage {
    /// Parse a user-facing stage name.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "committed" | "commit" => Some(Self::Committed),
            "dom" | "ready" => Some(Self::Dom),
            "settled" | "load" | "full" => Some(Self::Settled),
            _ => None,
        }
    }
}

/// Page-local keys supported by [`Request::Press`]. Keeping this typed avoids
/// silently accepting keys whose browser default behavior cannot be reproduced
/// by headless DOM automation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PressKey {
    Tab,
    Enter,
    Escape,
    ArrowLeft,
    ArrowRight,
}

impl PressKey {
    /// Parse a user-facing key name case-insensitively.
    pub fn parse(value: &str) -> Option<Self> {
        if value.eq_ignore_ascii_case("tab") {
            Some(Self::Tab)
        } else if value.eq_ignore_ascii_case("enter") || value.eq_ignore_ascii_case("return") {
            Some(Self::Enter)
        } else if value.eq_ignore_ascii_case("escape") || value.eq_ignore_ascii_case("esc") {
            Some(Self::Escape)
        } else if value.eq_ignore_ascii_case("arrowleft") {
            Some(Self::ArrowLeft)
        } else if value.eq_ignore_ascii_case("arrowright") {
            Some(Self::ArrowRight)
        } else {
            None
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tab => "Tab",
            Self::Enter => "Enter",
            Self::Escape => "Escape",
            Self::ArrowLeft => "ArrowLeft",
            Self::ArrowRight => "ArrowRight",
        }
    }
}

/// How an opened window is shown. Built for agent verification flows
/// that must not steal the user's focus.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenMode {
    /// Map and request focus (`present`). What a human asked for.
    #[default]
    Normal,
    /// Map the window but do not request activation: it appears in the
    /// WM layout, renders normally (eval/shot work), and focus stays
    /// where it is. Compositor policy has the final say; pair with a
    /// WM rule on `--app-id` to also keep it off the current workspace.
    Background,
    /// Never map a toplevel. The WebView lives offscreen in a
    /// `gtk::OffscreenHolder`-less window kept unrealized; rendering is
    /// driven by WebKit itself, so eval/goto/upload work and `shot`
    /// captures the page. Invisible to the WM entirely.
    Headless,
}

/// What to do with a page's virtual clock (see [`Request::Clock`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClockAction {
    /// Freeze virtual time. rAF stops firing, timers stop expiring,
    /// `performance.now()`/`Date.now()`/`new Date()` stop advancing, and
    /// running CSS/WAAPI animations are paused at the current virtual time.
    Pause,
    /// Return to real time. Wrapped clocks resume advancing from the
    /// current virtual time (monotonic: no backwards jumps).
    Resume,
    /// Advance a paused clock by `ms` virtual milliseconds: due timers
    /// fire, one rAF batch runs per 16.67 ms tick, and CSS/WAAPI
    /// currentTime advances by the same amount. Deterministic.
    Step,
    /// Pause and set absolute virtual time to `ms` (milliseconds since
    /// the clock was installed). Stepping semantics as `step`, from
    /// the current virtual time; going backwards is an error.
    Set,
    /// Report the clock's state without changing it.
    Status,
    /// Replace `Math.random` with a deterministic PRNG (mulberry32)
    /// seeded from `seed`. Applies immediately to the current page and
    /// persists for future loads in the same window (installed from
    /// document start, before page scripts can capture the native
    /// PRNG). Same seed + same virtual timeline => identical
    /// `Math.random()` sequences across loads. Without `seed`, pages
    /// keep native `Math.random`.
    Seed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdblockCmd {
    /// Enable blocking (persisted to config).
    On,
    /// Disable blocking (persisted to config).
    Off,
    /// Report current state.
    Status,
    /// Re-download filter lists and recompile.
    Update,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
// This is a public, serde-facing wire enum. Boxing one success field only to
// shrink the rare error value would make every client API less direct while
// leaving the serialized protocol unchanged.
#[allow(clippy::large_enum_variant)]
pub enum Response {
    Ok {
        #[serde(skip_serializing_if = "Option::is_none")]
        window: Option<WindowInfo>,
        #[serde(skip_serializing_if = "Option::is_none")]
        windows: Option<Vec<WindowInfo>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        adblock: Option<AdblockStatus>,
        /// Eval result (JSON), or ping capability info.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<serde_json::Value>,
        /// File path produced by a screenshot.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        /// Base64 payload for transports that do not share a filesystem.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        data: Option<String>,
    },
    Err {
        message: String,
    },
}

impl Response {
    pub fn ok() -> Self {
        Response::Ok {
            window: None,
            windows: None,
            adblock: None,
            value: None,
            path: None,
            data: None,
        }
    }
    pub fn window(w: WindowInfo) -> Self {
        let mut r = Response::ok();
        if let Response::Ok { window, .. } = &mut r {
            *window = Some(w);
        }
        r
    }
    pub fn windows(ws: Vec<WindowInfo>) -> Self {
        let mut r = Response::ok();
        if let Response::Ok { windows, .. } = &mut r {
            *windows = Some(ws);
        }
        r
    }
    pub fn adblock(status: AdblockStatus) -> Self {
        let mut r = Response::ok();
        if let Response::Ok { adblock, .. } = &mut r {
            *adblock = Some(status);
        }
        r
    }
    pub fn value(v: serde_json::Value) -> Self {
        let mut r = Response::ok();
        if let Response::Ok { value, .. } = &mut r {
            *value = Some(v);
        }
        r
    }
    pub fn path(p: impl Into<String>) -> Self {
        let mut r = Response::ok();
        if let Response::Ok { path, .. } = &mut r {
            *path = Some(p.into());
        }
        r
    }
    pub fn data(payload: impl Into<String>) -> Self {
        let mut r = Response::ok();
        if let Response::Ok { data, .. } = &mut r {
            *data = Some(payload.into());
        }
        r
    }
    pub fn err(message: impl Into<String>) -> Self {
        Response::Err {
            message: message.into(),
        }
    }
}

/// First frame sent on every TCP connection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthRequest {
    pub token: String,
}

/// Authentication reply. An error is always followed by connection close.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AuthReply {
    Ok,
    Err { message: String },
}

/// Canonical padded RFC 4648 base64 used by inline binary fields.
pub mod base64 {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;

    pub fn encode(data: &[u8]) -> String {
        STANDARD.encode(data)
    }

    pub fn decode(input: &str) -> Result<Vec<u8>, String> {
        STANDARD.decode(input).map_err(|error| error.to_string())
    }
}

/// Event kinds the daemon emits (see [`Event::event`]; `subscribed`
/// is the ack, not a filterable kind). Shared so client-side filter
/// validation cannot drift from the daemon.
pub const EVENT_KINDS: &[&str] = &["load", "console", "download", "window", "expect"];

/// One pushed event on a subscribed connection (see
/// [`Request::Subscribe`]). Serialized as a JSON line, tagged
/// `"event"` so a subscriber can tell events from the initial
/// [`Response`] if it ever multiplexes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Event kind: `subscribed` (the ack, seq 0), `load` (lifecycle:
    /// state=started|committed|finished|failed), `console` (a captured
    /// console/exception/network entry), `download`
    /// (state=finished|failed), `window` (state=opened|closed|focused).
    pub event: String,
    /// Strictly monotonic per connection, starting at 0 for the
    /// `subscribed` ack. A gap means the daemon dropped this client
    /// (it never silently skips), so gaps are impossible to observe:
    /// the connection dies instead.
    pub seq: u64,
    /// Window the event belongs to, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_id: Option<u64>,
    /// Milliseconds since UNIX epoch, stamped at emit time.
    pub ts_ms: u64,
    /// Kind-specific payload (load state, console entry, ...).
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub data: serde_json::Value,
}

/// State of the built-in content blocker, as reported by the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdblockStatus {
    pub enabled: bool,
    /// Compiled content-blocker rules currently active.
    pub rules: usize,
    /// Human-readable description of the filter source.
    pub source: String,
    /// True while WebKit is compiling a ruleset.
    pub compiling: bool,
    /// True while filter lists are being re-downloaded.
    pub updating: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowInfo {
    pub id: u64,
    pub url: String,
    pub title: String,
    /// True when this window currently has WM focus. Lets automation
    /// clients find "the window the user is looking at".
    #[serde(default)]
    pub focused: bool,
    /// True when the window's WebView has been discarded to save RAM;
    /// it restores automatically on focus.
    #[serde(default)]
    pub suspended: bool,
    /// Wayland app_id the window was opened with (`--app-id`), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    /// How the window was opened (normal/background/headless). `focus`
    /// promotes a window to normal.
    #[serde(default, skip_serializing_if = "is_normal")]
    pub mode: OpenMode,
    /// Last WebKit web-process termination observed for this window, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web_process_terminated: Option<Box<WebProcessTerminationInfo>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebProcessTerminationInfo {
    /// Stable, machine-readable reason: crashed, oom, or terminated.
    pub reason: String,
    /// Human-readable description suitable for diagnostics.
    pub message: String,
    /// Best-known URL at the time the web process died.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
}

fn is_normal(mode: &OpenMode) -> bool {
    *mode == OpenMode::Normal
}

fn is_false(v: &bool) -> bool {
    !*v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_parser_distinguishes_tcp_and_unix() {
        assert_eq!(
            parse_endpoint("tcp://127.0.0.1:8741"),
            Ok(Endpoint::Tcp("127.0.0.1:8741".into()))
        );
        assert_eq!(
            parse_endpoint("[::1]:8741"),
            Ok(Endpoint::Tcp("[::1]:8741".into()))
        );
        assert_eq!(
            parse_endpoint("tcp://daemon.internal:9000"),
            Ok(Endpoint::Tcp("daemon.internal:9000".into()))
        );
        assert_eq!(
            parse_endpoint("unix:///run/user/1000/hwatu.sock"),
            Ok(Endpoint::Unix(PathBuf::from("/run/user/1000/hwatu.sock")))
        );
        assert_eq!(
            parse_endpoint("/tmp/hwatu.sock"),
            Ok(Endpoint::Unix(PathBuf::from("/tmp/hwatu.sock")))
        );
    }

    #[test]
    fn endpoint_parser_rejects_ambiguous_or_malformed_tcp() {
        for invalid in [
            "",
            "tcp://localhost",
            "tcp://localhost:0",
            "localhost:not-a-port",
            "[::1]",
            "tcp://[::1]:8741:9",
            "tcp://local host:8741",
            "http://localhost:8741",
            "unix://",
        ] {
            assert!(
                parse_endpoint(invalid).is_err(),
                "malformed endpoint unexpectedly parsed: {invalid:?}"
            );
        }
    }

    #[test]
    fn authentication_frames_have_stable_wire_shapes() {
        let request = AuthRequest {
            token: "correct horse battery staple".into(),
        };
        let wire = serde_json::to_string(&request).unwrap();
        assert_eq!(wire, r#"{"token":"correct horse battery staple"}"#);
        assert_eq!(serde_json::from_str::<AuthRequest>(&wire).unwrap(), request);

        assert_eq!(
            serde_json::to_string(&AuthReply::Ok).unwrap(),
            r#"{"status":"ok"}"#
        );
        let denied = AuthReply::Err {
            message: "authentication failed".into(),
        };
        assert_eq!(
            serde_json::to_string(&denied).unwrap(),
            r#"{"status":"err","message":"authentication failed"}"#
        );
        assert_eq!(
            serde_json::from_str::<AuthReply>(
                r#"{"status":"err","message":"authentication failed"}"#
            )
            .unwrap(),
            denied
        );
    }

    #[test]
    fn authentication_token_bounds_are_enforced() {
        assert!(validate_token(&"x".repeat(MIN_TOKEN_BYTES - 1)).is_err());
        assert!(validate_token(&"x".repeat(MIN_TOKEN_BYTES)).is_ok());
        assert!(validate_token(&"x".repeat(MAX_TOKEN_BYTES)).is_ok());
        assert!(validate_token(&"x".repeat(MAX_TOKEN_BYTES + 1)).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn token_file_must_be_small_private_and_regular() {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

        let path = std::env::temp_dir().join(format!(
            "hwatu-token-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let token = "0123456789abcdef0123456789abcdef";
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        writeln!(file, "{token}").unwrap();
        drop(file);

        assert_eq!(load_token_file(&path).unwrap(), token);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        assert!(load_token_file(&path)
            .unwrap_err()
            .contains("group or other"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn inline_base64_is_canonical_and_strict() {
        for (plain, encoded) in [
            (b"".as_slice(), ""),
            (b"f".as_slice(), "Zg=="),
            (b"fo".as_slice(), "Zm8="),
            (b"foo".as_slice(), "Zm9v"),
            (b"foobar".as_slice(), "Zm9vYmFy"),
        ] {
            assert_eq!(base64::encode(plain), encoded);
            assert_eq!(base64::decode(encoded).unwrap(), plain);
        }

        for invalid in ["Zg", "Zg=", "Zg===", "Zh==", "Zm9v\n"] {
            assert!(
                base64::decode(invalid).is_err(),
                "non-canonical base64 unexpectedly decoded: {invalid:?}"
            );
        }
    }

    #[test]
    fn maximum_inline_payload_fits_in_one_protocol_frame() {
        let encoded_len = INLINE_MAX_BYTES.div_ceil(3) * 4;
        const ENVELOPE_ALLOWANCE: usize = 1024 * 1024;
        assert!(encoded_len + ENVELOPE_ALLOWANCE < MAX_FRAME_BYTES);
    }

    #[test]
    fn bounded_frame_helpers_preserve_following_frames() {
        let first = AuthReply::Ok;
        let second = Response::value(serde_json::json!({ "ready": true }));
        let mut wire = encode_frame(&first, MAX_AUTH_FRAME_BYTES).unwrap();
        wire.extend(encode_frame(&second, MAX_FRAME_BYTES).unwrap());

        let mut reader = std::io::BufReader::with_capacity(3, wire.as_slice());
        assert_eq!(
            read_frame(&mut reader, MAX_AUTH_FRAME_BYTES).unwrap(),
            Some(br#"{"status":"ok"}"#.to_vec())
        );
        let response = read_frame(&mut reader, MAX_FRAME_BYTES).unwrap().unwrap();
        assert!(matches!(
            serde_json::from_slice::<Response>(&response).unwrap(),
            Response::Ok { value: Some(_), .. }
        ));
        assert_eq!(read_frame(&mut reader, MAX_FRAME_BYTES).unwrap(), None);
    }

    #[test]
    fn bounded_frame_reader_rejects_oversize_and_truncated_frames() {
        let mut exact = std::io::BufReader::new(&b"abc\n"[..]);
        assert_eq!(read_frame(&mut exact, 4).unwrap(), Some(b"abc".to_vec()));

        let mut oversize = std::io::BufReader::new(&b"abcd\n"[..]);
        assert_eq!(
            read_frame(&mut oversize, 4).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );

        let mut no_newline = std::io::BufReader::new(&b"abc"[..]);
        assert_eq!(
            read_frame(&mut no_newline, 4).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }

    #[test]
    fn old_payload_requests_and_responses_default_new_fields() {
        let Request::Screenshot { data, .. } =
            serde_json::from_str::<Request>(r#"{"cmd":"screenshot","full":true}"#).unwrap()
        else {
            panic!("old screenshot request failed to parse");
        };
        assert!(!data);

        let Request::Upload { data, .. } = serde_json::from_str::<Request>(
            r#"{"cmd":"upload","selector":"input","path":"/tmp/file"}"#,
        )
        .unwrap() else {
            panic!("old upload request failed to parse");
        };
        assert_eq!(data, None);

        let Response::Ok { data, .. } =
            serde_json::from_str::<Response>(r#"{"status":"ok"}"#).unwrap()
        else {
            panic!("old response failed to parse");
        };
        assert_eq!(data, None);
    }

    /// An old client's check request (url as a bare string, no
    /// render/base fields) must deserialize unchanged: the wire
    /// invariant that lets sessions ship incrementally.
    #[test]
    fn old_client_check_still_parses() {
        let old = r#"{"cmd":"check","url":"http://localhost:3000","shot":true}"#;
        let Ok(Request::Check {
            url, render, base, ..
        }) = serde_json::from_str::<Request>(old)
        else {
            panic!("old check request failed to parse");
        };
        assert_eq!(url.as_deref(), Some("http://localhost:3000"));
        assert_eq!(render, None);
        assert_eq!(base, None);
    }

    #[test]
    fn press_key_parses_and_roundtrips_on_the_wire() {
        assert_eq!(PressKey::parse("Tab"), Some(PressKey::Tab));
        assert_eq!(PressKey::parse("enter"), Some(PressKey::Enter));
        assert_eq!(PressKey::parse("Return"), Some(PressKey::Enter));
        assert_eq!(PressKey::parse("Escape"), Some(PressKey::Escape));
        assert_eq!(PressKey::parse("esc"), Some(PressKey::Escape));
        assert_eq!(PressKey::parse("ArrowLeft"), Some(PressKey::ArrowLeft));
        assert_eq!(PressKey::parse("arrowright"), Some(PressKey::ArrowRight));

        let request = Request::Press {
            id: Some(4),
            key: PressKey::Tab,
            timeout_ms: Some(750),
        };
        let wire = serde_json::to_string(&request).unwrap();
        assert_eq!(
            wire,
            r#"{"cmd":"press","id":4,"key":"tab","timeout_ms":750}"#
        );
        let Request::Press { id, key, .. } = serde_json::from_str::<Request>(&wire).unwrap() else {
            panic!("press failed to roundtrip");
        };
        assert_eq!(id, Some(4));
        assert_eq!(key, PressKey::Tab);

        for (key, wire_name) in [
            (PressKey::ArrowLeft, "arrowleft"),
            (PressKey::ArrowRight, "arrowright"),
        ] {
            let request = Request::Press {
                id: None,
                key,
                timeout_ms: None,
            };
            let wire = serde_json::to_string(&request).unwrap();
            assert_eq!(
                wire,
                format!(r#"{{"cmd":"press","id":null,"key":"{wire_name}","timeout_ms":null}}"#)
            );
            let Request::Press {
                key: roundtripped, ..
            } = serde_json::from_str::<Request>(&wire).unwrap()
            else {
                panic!("arrow press failed to roundtrip");
            };
            assert_eq!(roundtripped, key);
        }
        assert!(request.is_batch_action());
    }

    /// A render-check roundtrips through the wire format, and the
    /// absent url is omitted from the JSON entirely (so an old daemon
    /// answers a new client's render attempt with a clean missing-
    /// field error instead of misrouting it).
    #[test]
    fn render_check_roundtrips_and_omits_url() {
        let req = Request::Check {
            url: None,
            render: Some("<h1>hi</h1>".into()),
            base: Some("http://localhost:3000/".into()),
            eval: None,
            shot: false,
            shot_path: None,
            shot_data: false,
            full: false,
            baseline: None,
            baseline_data: None,
            tolerance: None,
            heatmap: None,
            heatmap_data: false,
            until: LoadStage::Dom,
            keep: false,
            timeout_ms: None,
            viewports: vec![],
            baseline_dir: None,
        };
        let wire = serde_json::to_string(&req).unwrap();
        assert!(
            !wire.contains("\"url\""),
            "absent url must be omitted: {wire}"
        );
        assert!(wire.contains("\"render\""));
        let Ok(Request::Check { render, base, .. }) = serde_json::from_str::<Request>(&wire) else {
            panic!("render check failed to roundtrip");
        };
        assert_eq!(render.as_deref(), Some("<h1>hi</h1>"));
        assert_eq!(base.as_deref(), Some("http://localhost:3000/"));
    }

    #[test]
    fn old_window_info_defaults_recovery_fields() {
        let old = r#"{"id":7,"url":"","title":"","focused":false,"suspended":false}"#;
        let info: WindowInfo = serde_json::from_str(old).expect("old WindowInfo parses");

        assert_eq!(info.web_process_terminated, None);
    }

    #[test]
    fn window_info_serializes_recoverable_crash_state() {
        let info = WindowInfo {
            id: 7,
            url: "https://example.test/sign-up".into(),
            title: String::new(),
            focused: true,
            suspended: false,
            app_id: None,
            mode: OpenMode::Normal,
            web_process_terminated: Some(Box::new(WebProcessTerminationInfo {
                reason: "oom".into(),
                message: "was killed (out of memory)".into(),
                url: "https://example.test/sign-up".into(),
            })),
        };

        let json = serde_json::to_value(&info).expect("WindowInfo serializes");

        assert_eq!(json["web_process_terminated"]["reason"], "oom");
        assert_eq!(
            json["web_process_terminated"]["url"],
            "https://example.test/sign-up"
        );
    }

    /// A viewport-sweep check roundtrips through the wire format; an
    /// empty sweep is omitted from the JSON entirely so old daemons
    /// keep parsing new clients' plain checks unchanged.
    #[test]
    fn viewport_check_roundtrips_and_empty_is_omitted() {
        let req = Request::Check {
            url: Some("http://localhost:3000".into()),
            render: None,
            base: None,
            eval: None,
            shot: false,
            shot_path: None,
            shot_data: false,
            full: false,
            baseline: None,
            baseline_data: None,
            tolerance: None,
            heatmap: None,
            heatmap_data: false,
            until: LoadStage::default(),
            keep: false,
            timeout_ms: None,
            viewports: vec![Viewport { w: 360, h: 640 }, Viewport { w: 1920, h: 1080 }],
            baseline_dir: Some("/tmp/base".into()),
        };
        let wire = serde_json::to_string(&req).unwrap();
        assert!(wire.contains("\"viewports\""));
        assert!(wire.contains("\"baseline_dir\""));
        let Ok(Request::Check {
            viewports,
            baseline_dir,
            ..
        }) = serde_json::from_str::<Request>(&wire)
        else {
            panic!("viewport check failed to roundtrip");
        };
        assert_eq!(
            viewports,
            vec![Viewport { w: 360, h: 640 }, Viewport { w: 1920, h: 1080 }]
        );
        assert_eq!(baseline_dir.as_deref(), Some("/tmp/base"));

        // No sweep: neither field appears on the wire, and an old
        // request without them still parses (defaults).
        let plain = Request::Check {
            url: Some("http://localhost:3000".into()),
            render: None,
            base: None,
            eval: None,
            shot: false,
            shot_path: None,
            shot_data: false,
            full: false,
            baseline: None,
            baseline_data: None,
            tolerance: None,
            heatmap: None,
            heatmap_data: false,
            until: LoadStage::default(),
            keep: false,
            timeout_ms: None,
            viewports: vec![],
            baseline_dir: None,
        };
        let wire = serde_json::to_string(&plain).unwrap();
        assert!(
            !wire.contains("viewports"),
            "empty sweep must be omitted: {wire}"
        );
        assert!(!wire.contains("baseline_dir"));
        let Ok(Request::Check {
            viewports,
            baseline_dir,
            ..
        }) = serde_json::from_str::<Request>(r#"{"cmd":"check","url":"x.test"}"#)
        else {
            panic!("plain check failed to parse");
        };
        assert!(viewports.is_empty());
        assert_eq!(baseline_dir, None);
    }

    /// Viewport size parsing: valid forms, bounds, and list errors.
    #[test]
    fn viewport_parsing() {
        assert_eq!(
            Viewport::parse("360x640"),
            Some(Viewport { w: 360, h: 640 })
        );
        assert_eq!(
            Viewport::parse(" 800X600 "),
            Some(Viewport { w: 800, h: 600 })
        );
        assert_eq!(Viewport::parse("0x640"), None);
        assert_eq!(Viewport::parse("360x"), None);
        assert_eq!(Viewport::parse("360"), None);
        assert_eq!(Viewport::parse("-1x640"), None);
        assert_eq!(Viewport::parse("99999x640"), None);

        let sizes = Viewport::parse_list("360x640,768x1024,1920x1080").unwrap();
        assert_eq!(sizes.len(), 3);
        assert_eq!(sizes[2].label(), "1920x1080");
        assert!(Viewport::parse_list("360x640,banana").is_err());
        assert!(Viewport::parse_list("").unwrap().is_empty());
    }

    /// A bare `{"cmd":"net"}` (all defaults) is valid, and a full Net
    /// request roundtrips through the wire format. Absent fields keep
    /// the line-delimited JSON back-compat contract: an old daemon
    /// answers a new client's `net` with a clean "unknown variant"
    /// error (which the CLI turns into a restart hint), and an old
    /// client never sends `net` at all.
    #[test]
    fn net_roundtrips_with_defaults() {
        let Ok(Request::Net { id, clear, limit }) =
            serde_json::from_str::<Request>(r#"{"cmd":"net"}"#)
        else {
            panic!("bare net failed to parse");
        };
        assert_eq!(id, None);
        assert!(!clear);
        assert_eq!(limit, None);

        let req = Request::Net {
            id: Some(4),
            clear: true,
            limit: Some(50),
        };
        let wire = serde_json::to_string(&req).unwrap();
        assert!(wire.contains("\"cmd\":\"net\""));
        let Ok(Request::Net { id, clear, limit }) = serde_json::from_str::<Request>(&wire) else {
            panic!("net failed to roundtrip");
        };
        assert_eq!(id, Some(4));
        assert!(clear);
        assert_eq!(limit, Some(50));
    }

    #[test]
    fn paste_roundtrips_with_selector_or_ref() {
        let req = Request::Paste {
            id: Some(9),
            selector: Some("textarea".into()),
            nth: Some(1),
            contains: Some("Bio".into()),
            r#ref: None,
            timeout_ms: Some(2500),
        };
        let wire = serde_json::to_string(&req).unwrap();
        assert!(wire.contains("\"cmd\":\"paste\""));
        let Ok(Request::Paste {
            id,
            selector,
            nth,
            contains,
            r#ref,
            timeout_ms,
        }) = serde_json::from_str::<Request>(&wire)
        else {
            panic!("paste failed to roundtrip");
        };
        assert_eq!(id, Some(9));
        assert_eq!(selector.as_deref(), Some("textarea"));
        assert_eq!(nth, Some(1));
        assert_eq!(contains.as_deref(), Some("Bio"));
        assert_eq!(r#ref, None);
        assert_eq!(timeout_ms, Some(2500));

        let Ok(Request::Paste {
            selector, r#ref, ..
        }) = serde_json::from_str::<Request>(r#"{"cmd":"paste","ref":7}"#)
        else {
            panic!("paste ref failed to parse");
        };
        assert!(selector.is_none());
        assert_eq!(r#ref, Some(7));
    }

    /// Subscribe roundtrips with and without filters; a bare
    /// `{"cmd":"subscribe"}` (defaults) is valid so shell one-liners
    /// stay terse.
    #[test]
    fn subscribe_roundtrips_with_defaults() {
        let Ok(Request::Subscribe { kinds, window }) =
            serde_json::from_str::<Request>(r#"{"cmd":"subscribe"}"#)
        else {
            panic!("bare subscribe failed to parse");
        };
        assert_eq!(kinds, None);
        assert_eq!(window, None);

        let req = Request::Subscribe {
            kinds: Some(vec!["load".into(), "console".into()]),
            window: Some(7),
        };
        let wire = serde_json::to_string(&req).unwrap();
        let Ok(Request::Subscribe { kinds, window }) = serde_json::from_str::<Request>(&wire)
        else {
            panic!("subscribe failed to roundtrip");
        };
        assert_eq!(
            kinds.as_deref(),
            Some(&["load".to_string(), "console".to_string()][..])
        );
        assert_eq!(window, Some(7));
    }

    #[test]
    fn batch_request_roundtrips_and_validates_allowlist() {
        let req = Request::Batch {
            actions: vec![
                Request::Eval {
                    id: Some(1),
                    js: "return document.title".into(),
                    timeout_ms: Some(1000),
                },
                Request::Click {
                    id: Some(1),
                    selector: Some("button".into()),
                    nth: None,
                    contains: Some("Save".into()),
                    r#ref: None,
                    trusted: false,
                    timeout_ms: None,
                },
                Request::Expect {
                    id: Some(1),
                    selector: ".done".into(),
                    nth: None,
                    contains: None,
                    text: Some("Saved".into()),
                    absent: false,
                    visible: true,
                    timeout_ms: Some(2000),
                    watch: false,
                },
            ],
        };
        let wire = serde_json::to_string(&req).unwrap();
        assert!(wire.contains("\"cmd\":\"batch\""));
        let Request::Batch { actions } = serde_json::from_str::<Request>(&wire).unwrap() else {
            panic!("batch failed to roundtrip");
        };
        assert_eq!(actions.len(), 3);
        Request::validate_batch(&actions).unwrap();
    }

    #[test]
    fn uses_eval_covers_direct_check_and_batch_surfaces() {
        assert!(Request::Eval {
            id: None,
            js: "return document.cookie".into(),
            timeout_ms: None,
        }
        .uses_eval());

        let mut check = Request::Check {
            url: Some("https://example.test".into()),
            render: None,
            base: None,
            eval: None,
            shot: false,
            shot_path: None,
            shot_data: false,
            full: false,
            baseline: None,
            baseline_data: None,
            tolerance: None,
            heatmap: None,
            heatmap_data: false,
            until: LoadStage::default(),
            keep: false,
            timeout_ms: None,
            viewports: vec![],
            baseline_dir: None,
        };
        assert!(!check.uses_eval());
        if let Request::Check { eval, .. } = &mut check {
            *eval = Some("".into());
        }
        assert!(!check.uses_eval());
        if let Request::Check { eval, .. } = &mut check {
            *eval = Some("localStorage.secret".into());
        }
        assert!(check.uses_eval());

        assert!(!Request::Navigate {
            id: Some(1),
            url: "https://example.test".into(),
            wait: true,
            until: LoadStage::default(),
            timeout_ms: None,
        }
        .uses_eval());

        assert!(Request::Batch {
            actions: vec![
                Request::Navigate {
                    id: Some(1),
                    url: "https://example.test".into(),
                    wait: true,
                    until: LoadStage::default(),
                    timeout_ms: None,
                },
                check,
            ],
        }
        .uses_eval());

        assert!(Request::Batch {
            actions: vec![Request::Batch {
                actions: vec![Request::Eval {
                    id: Some(1),
                    js: "document.cookie".into(),
                    timeout_ms: None,
                }],
            }],
        }
        .uses_eval());
    }

    #[test]
    fn batch_validation_rejects_nested_oversized_and_unsupported_actions() {
        assert!(Request::validate_batch(&[])
            .unwrap_err()
            .contains("at least one"));

        let too_many = vec![
            Request::Snapshot {
                id: None,
                diff: false,
                rect: false,
                budget: None,
                timeout_ms: None,
            };
            BATCH_MAX_ACTIONS + 1
        ];
        assert!(Request::validate_batch(&too_many)
            .unwrap_err()
            .contains("cap"));

        let nested = vec![Request::Batch {
            actions: vec![Request::Snapshot {
                id: None,
                diff: false,
                rect: false,
                budget: None,
                timeout_ms: None,
            }],
        }];
        assert!(Request::validate_batch(&nested)
            .unwrap_err()
            .contains("nested"));

        let unsupported = vec![Request::Close { id: 1 }];
        assert!(Request::validate_batch(&unsupported)
            .unwrap_err()
            .contains("unsupported"));

        let watch = vec![Request::Expect {
            id: None,
            selector: "main".into(),
            nth: None,
            contains: None,
            text: None,
            absent: false,
            visible: false,
            timeout_ms: None,
            watch: true,
        }];
        assert!(Request::validate_batch(&watch)
            .unwrap_err()
            .contains("watch"));
    }

    #[test]
    fn batch_result_wire_shape_exposes_partial_execution() {
        let result = BatchResult {
            complete: false,
            executed: 2,
            failed_at: Some(1),
            steps: vec![
                BatchStepResult {
                    index: 0,
                    action: "eval".into(),
                    status: BatchStepStatus::Ok,
                    response: Some(Response::value(serde_json::json!(1))),
                    error: None,
                    skipped_reason: None,
                },
                BatchStepResult {
                    index: 1,
                    action: "click".into(),
                    status: BatchStepStatus::Error,
                    response: Some(Response::err("button not found")),
                    error: Some("button not found".into()),
                    skipped_reason: None,
                },
                BatchStepResult {
                    index: 2,
                    action: "type".into(),
                    status: BatchStepStatus::Skipped,
                    response: None,
                    error: None,
                    skipped_reason: Some("not run after step 1 failed".into()),
                },
            ],
        };
        let response = Response::value(serde_json::json!({ "batch": result }));
        let wire = serde_json::to_string(&response).unwrap();
        assert!(wire.contains("\"complete\":false"));
        assert!(wire.contains("\"failed_at\":1"));
        assert!(wire.contains("\"status\":\"skipped\""));
        let Response::Ok { value: Some(v), .. } = serde_json::from_str::<Response>(&wire).unwrap()
        else {
            panic!("batch response failed to parse");
        };
        assert_eq!(v["batch"]["executed"], 2);
        assert_eq!(v["batch"]["steps"][1]["error"], "button not found");
    }

    /// Snapshot keeps the wire back-compat contract around `diff`: a
    /// bare `{"cmd":"snapshot"}` from an old client parses with
    /// `diff: false`, and a new client's default (non-diff) snapshot
    /// omits the field entirely so an old daemon still parses it.
    #[test]
    fn snapshot_diff_wire_compat() {
        let Ok(Request::Snapshot { id, diff, .. }) =
            serde_json::from_str::<Request>(r#"{"cmd":"snapshot"}"#)
        else {
            panic!("bare snapshot failed to parse");
        };
        assert_eq!(id, None);
        assert!(!diff, "absent diff must default to false");

        let plain = Request::Snapshot {
            id: Some(2),
            diff: false,
            rect: false,
            budget: None,
            timeout_ms: None,
        };
        let wire = serde_json::to_string(&plain).unwrap();
        assert!(
            !wire.contains("diff"),
            "non-diff snapshot must omit the field for old daemons: {wire}"
        );
        assert!(!wire.contains("rect"));
        assert!(
            !wire.contains("budget"),
            "absent budget must stay off the wire"
        );

        let diffing = Request::Snapshot {
            id: Some(2),
            diff: true,
            rect: true,
            budget: Some(2000),
            timeout_ms: None,
        };
        let wire = serde_json::to_string(&diffing).unwrap();
        assert!(wire.contains("\"diff\":true"));
        assert!(wire.contains("\"rect\":true"));
        assert!(wire.contains("\"budget\":2000"));
        let Ok(Request::Snapshot { diff, .. }) = serde_json::from_str::<Request>(&wire) else {
            panic!("diff snapshot failed to roundtrip");
        };
        assert!(diff);
    }

    /// Events serialize with seq + window_id and omit empty payloads;
    /// the `event` tag is what stream consumers dispatch on.
    #[test]
    fn event_wire_shape() {
        let e = Event {
            event: "load".into(),
            seq: 3,
            window_id: Some(9),
            ts_ms: 1234,
            data: serde_json::json!({ "state": "finished" }),
        };
        let wire = serde_json::to_string(&e).unwrap();
        assert!(wire.contains("\"event\":\"load\""));
        assert!(wire.contains("\"seq\":3"));
        assert!(wire.contains("\"window_id\":9"));
        let back: Event = serde_json::from_str(&wire).unwrap();
        assert_eq!(back.data["state"], "finished");

        // Null payloads and absent window ids are omitted, not "null".
        let bare = Event {
            event: "subscribed".into(),
            seq: 0,
            window_id: None,
            ts_ms: 0,
            data: serde_json::Value::Null,
        };
        let wire = serde_json::to_string(&bare).unwrap();
        assert!(!wire.contains("window_id"));
        assert!(!wire.contains("data"));
    }
}
