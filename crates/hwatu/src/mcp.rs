//! `hwatu mcp`: a Model Context Protocol server over stdio.
//!
//! A thin translation layer: MCP `tools/call` requests become the same
//! one-line-JSON socket requests the CLI sends; the daemon stays the
//! source of truth. No SDK, no extra dependencies: MCP's stdio
//! transport is newline-delimited JSON-RPC 2.0, which serde_json
//! handles directly.
//!
//! Register it in an MCP client as command `hwatu`, args `["mcp"]`.
//! The daemon is autostarted on the first tool call, like every other
//! `hwatu` invocation.

use hwatu_ipc::{ClockAction, LoadStage, OpenMode, Request, Response};
use serde_json::{json, Value};
use std::io::{BufRead, Write};

const PROTOCOL_VERSION: &str = "2024-11-05";

/// Serve MCP over stdio until stdin closes. Returns the process exit code.
pub fn run() -> i32 {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                send(
                    &stdout,
                    &jsonrpc_error(Value::Null, -32700, &format!("parse error: {e}")),
                );
                continue;
            }
        };
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(Value::Null);

        // Notifications (no id) never get a response.
        let Some(id) = id else { continue };

        let reply = match method {
            "initialize" => jsonrpc_result(id, initialize_result(&params)),
            "ping" => jsonrpc_result(id, json!({})),
            "tools/list" => jsonrpc_result(id, json!({ "tools": tool_definitions() })),
            "tools/call" => match handle_tool_call(&params) {
                Ok(text) => jsonrpc_result(id, tool_text(&text, false)),
                Err(text) => jsonrpc_result(id, tool_text(&text, true)),
            },
            // Optional capabilities we don't provide.
            "resources/list" => jsonrpc_result(id, json!({ "resources": [] })),
            "prompts/list" => jsonrpc_result(id, json!({ "prompts": [] })),
            other => jsonrpc_error(id, -32601, &format!("method not found: {other}")),
        };
        send(&stdout, &reply);
    }
    0
}

fn send(stdout: &std::io::Stdout, msg: &Value) {
    let mut out = stdout.lock();
    let _ = serde_json::to_writer(&mut out, msg);
    let _ = out.write_all(b"\n");
    let _ = out.flush();
}

fn jsonrpc_result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn jsonrpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn initialize_result(params: &Value) -> Value {
    // Echo the client's protocol version when it names one we can
    // serve; the tools-only surface is stable across known revisions.
    let version = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or(PROTOCOL_VERSION);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": {
            "name": "hwatu",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "instructions": "hwatu drives a daemon-based WebKit browser for visual \
            verification: open pages (headless by default), read them as JSON \
            (snapshot), act (click/type_text by ref or selector), check errors \
            (console), and screenshot. When a tool call omits `id`, it targets \
            the window the previous call touched, so open -> snapshot -> click \
            chains never need ids. Use focus to show a window to the human \
            (e.g. after challenge reports a CAPTCHA)."
    })
}

/// MCP tool result content: one text block, flagged as error or not.
fn tool_text(text: &str, is_error: bool) -> Value {
    json!({
        "content": [{ "type": "text", "text": text }],
        "isError": is_error,
    })
}

/// `tools/call` -> daemon roundtrip -> response text.
fn handle_tool_call(params: &Value) -> Result<String, String> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or("missing tool name")?;
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    let request = build_request(name, &args)?;
    let response = transact(&request)?;
    match response {
        Response::Err { message } => Err(message),
        ok => serde_json::to_string(&ok).map_err(|e| e.to_string()),
    }
}

/// One request per connection, like the CLI.
fn transact(request: &Request) -> Result<Response, String> {
    let mut stream =
        crate::connect_or_spawn().map_err(|e| format!("cannot reach hwatu daemon: {e}"))?;
    let mut payload = serde_json::to_vec(request).map_err(|e| e.to_string())?;
    payload.push(b'\n');
    stream
        .write_all(&payload)
        .map_err(|e| format!("write failed: {e}"))?;
    let mut line = String::new();
    std::io::BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|e| format!("read failed: {e}"))?;
    serde_json::from_str(line.trim()).map_err(|e| format!("bad daemon response: {e} ({line:?})"))
}

// ---- argument extraction helpers ----------------------------------

fn opt_str(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_string)
}

fn req_str(args: &Value, key: &str) -> Result<String, String> {
    opt_str(args, key).ok_or_else(|| format!("missing required argument: {key}"))
}

fn opt_u64(args: &Value, key: &str) -> Option<u64> {
    args.get(key).and_then(Value::as_u64)
}

fn opt_u32(args: &Value, key: &str) -> Option<u32> {
    opt_u64(args, key).map(|v| v as u32)
}

fn opt_f64(args: &Value, key: &str) -> Option<f64> {
    args.get(key).and_then(Value::as_f64)
}

fn opt_bool(args: &Value, key: &str) -> Option<bool> {
    args.get(key).and_then(Value::as_bool)
}

/// Map an MCP tool call onto the wire [`Request`].
pub(crate) fn build_request(name: &str, args: &Value) -> Result<Request, String> {
    let id = opt_u64(args, "id");
    let timeout_ms = opt_u64(args, "timeout_ms");
    match name {
        "open" => {
            // MCP callers are agents: headless unless they say otherwise.
            let mode = match opt_str(args, "mode").as_deref() {
                None => OpenMode::Headless,
                Some("headless") => OpenMode::Headless,
                Some("background") => OpenMode::Background,
                Some("normal") | Some("focus") => OpenMode::Normal,
                Some(other) => {
                    return Err(format!(
                        "invalid mode {other:?} (want normal|background|headless)"
                    ))
                }
            };
            Ok(Request::Open {
                url: opt_str(args, "url"),
                app_id: opt_str(args, "app_id"),
                mode,
            })
        }
        "list_windows" => Ok(Request::List),
        "close" => Ok(Request::Close {
            id: opt_u64(args, "id").ok_or("missing required argument: id")?,
        }),
        "focus" => Ok(Request::Focus {
            id: opt_u64(args, "id").ok_or("missing required argument: id")?,
        }),
        "goto" => Ok(Request::Navigate {
            id,
            url: req_str(args, "url")?,
            wait: opt_bool(args, "wait").unwrap_or(true),
            until: parse_until(args)?,
            timeout_ms,
        }),
        "eval" => Ok(Request::Eval {
            id,
            js: req_str(args, "js")?,
            timeout_ms,
        }),
        "screenshot" => Ok(Request::Screenshot {
            id,
            path: opt_str(args, "path"),
            full: opt_bool(args, "full").unwrap_or(false),
        }),
        "wait_load" => Ok(Request::WaitLoad {
            id,
            until: parse_until(args)?,
            timeout_ms,
        }),
        "check" => Ok(Request::Check {
            url: req_str(args, "url")?,
            eval: opt_str(args, "eval"),
            shot: opt_bool(args, "shot").unwrap_or(false),
            shot_path: opt_str(args, "shot_path"),
            full: opt_bool(args, "full").unwrap_or(false),
            until: parse_until(args)?,
            keep: opt_bool(args, "keep").unwrap_or(false),
            timeout_ms,
        }),
        "snapshot" => Ok(Request::Snapshot { id, timeout_ms }),
        "expect" => Ok(Request::Expect {
            id,
            selector: req_str(args, "selector")?,
            nth: opt_u32(args, "nth"),
            contains: opt_str(args, "contains"),
            text: opt_str(args, "text"),
            absent: opt_bool(args, "absent").unwrap_or(false),
            timeout_ms,
        }),
        "click" => {
            let selector = opt_str(args, "selector");
            let r#ref = opt_u32(args, "ref");
            if selector.is_none() && r#ref.is_none() {
                return Err("click needs `selector` or `ref`".into());
            }
            Ok(Request::Click {
                id,
                selector,
                nth: opt_u32(args, "nth"),
                contains: opt_str(args, "contains"),
                r#ref,
                timeout_ms,
            })
        }
        "type_text" => {
            let selector = opt_str(args, "selector");
            let r#ref = opt_u32(args, "ref");
            if selector.is_none() && r#ref.is_none() {
                return Err("type_text needs `selector` or `ref`".into());
            }
            Ok(Request::Type {
                id,
                selector,
                nth: opt_u32(args, "nth"),
                contains: opt_str(args, "contains"),
                r#ref,
                text: req_str(args, "text")?,
                clear: opt_bool(args, "clear").unwrap_or(true),
                enter: opt_bool(args, "enter").unwrap_or(false),
                timeout_ms,
            })
        }
        "scroll" => Ok(Request::Scroll {
            id,
            selector: opt_str(args, "selector"),
            nth: opt_u32(args, "nth"),
            contains: opt_str(args, "contains"),
            to_y: opt_f64(args, "to_y"),
            by_pages: opt_f64(args, "by_pages"),
            timeout_ms,
        }),
        "console" => Ok(Request::Console {
            id,
            clear: opt_bool(args, "clear").unwrap_or(false),
            limit: opt_u64(args, "limit").map(|v| v as usize),
        }),
        "upload" => Ok(Request::Upload {
            id,
            selector: req_str(args, "selector")?,
            path: req_str(args, "path")?,
            timeout_ms,
        }),
        "challenge" => Ok(Request::Challenge {
            id,
            wait: opt_bool(args, "wait").unwrap_or(false),
            timeout_ms,
        }),
        "motion" => Ok(Request::Motion {
            id,
            observe: opt_bool(args, "observe").unwrap_or(false),
            observe_ms: opt_u64(args, "observe_ms"),
            timeout_ms,
        }),
        "clock" => {
            let action = match req_str(args, "action")?.as_str() {
                "pause" => ClockAction::Pause,
                "resume" => ClockAction::Resume,
                "step" => ClockAction::Step,
                "set" => ClockAction::Set,
                "seed" => ClockAction::Seed,
                "status" => ClockAction::Status,
                other => {
                    return Err(format!(
                        "unknown clock action {other:?} (want pause|resume|step|set|seed|status)"
                    ))
                }
            };
            let ms = opt_f64(args, "ms");
            let seed = opt_u64(args, "seed");
            if matches!(action, ClockAction::Step | ClockAction::Set) && ms.is_none() {
                return Err("clock step/set needs `ms`".into());
            }
            if matches!(action, ClockAction::Seed) && seed.is_none() {
                return Err("clock seed needs `seed`".into());
            }
            Ok(Request::Clock {
                id,
                action,
                ms,
                seed,
                timeout_ms,
            })
        }
        "seek" => {
            let time_ms = opt_f64(args, "time_ms");
            let progress = opt_f64(args, "progress");
            let resume = opt_bool(args, "resume").unwrap_or(false);
            if time_ms.is_none() && progress.is_none() && !resume {
                return Err("seek needs `time_ms`, `progress`, or `resume`".into());
            }
            Ok(Request::Seek {
                id,
                time_ms,
                progress,
                resume,
                timeout_ms,
            })
        }
        "diff" => {
            let other = opt_u64(args, "other");
            let baseline = opt_str(args, "baseline");
            if other.is_none() && baseline.is_none() {
                return Err("diff needs `other` (window id) or `baseline` (PNG path)".into());
            }
            Ok(Request::Diff {
                id: opt_u64(args, "id").ok_or("missing required argument: id")?,
                other,
                baseline,
                tolerance: opt_u64(args, "tolerance").map(|v| v.min(255) as u8),
                heatmap: opt_str(args, "heatmap"),
                full: opt_bool(args, "full").unwrap_or(false),
                timeout_ms,
            })
        }
        other => Err(format!("unknown tool: {other}")),
    }
}

/// Optional `until` load-stage argument, defaulting to settled.
fn parse_until(args: &Value) -> Result<LoadStage, String> {
    match opt_str(args, "until") {
        None => Ok(LoadStage::default()),
        Some(v) => LoadStage::parse(&v)
            .ok_or_else(|| format!("invalid until {v:?} (want committed|dom|settled)")),
    }
}

// ---- tool schemas --------------------------------------------------

/// Shorthand JSON-Schema property.
fn prop(ty: &str, desc: &str) -> Value {
    json!({ "type": ty, "description": desc })
}

/// Build one tool definition.
fn tool(name: &str, desc: &str, props: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": desc,
        "inputSchema": {
            "type": "object",
            "properties": props,
            "required": required,
        },
    })
}

const ID_DESC: &str = "Window id. Omit to target the window the last call touched \
    (or the focused/only window).";

pub(crate) fn tool_definitions() -> Vec<Value> {
    vec![
        tool(
            "open",
            "Open a browser window (headless by default: fully live but invisible \
             to the window manager). Returns the window id. ~15ms on a warm daemon.",
            json!({
                "url": prop("string", "URL to load (https:// implied; non-URL text becomes a web search). Omit for a blank launcher page."),
                "mode": { "type": "string", "enum": ["headless", "background", "normal"],
                    "description": "headless (default): no window at all. background: mapped in the WM but not focused. normal: mapped and focused." },
                "app_id": prop("string", "Wayland app_id, for the user's WM window rules."),
            }),
            &[],
        ),
        tool(
            "list_windows",
            "List all windows: id, url, title, focused, suspended, mode.",
            json!({}),
            &[],
        ),
        tool(
            "goto",
            "Navigate a window and (by default) wait for the load to settle.",
            json!({
                "id": prop("integer", ID_DESC),
                "url": prop("string", "URL to load."),
                "wait": prop("boolean", "Wait for the load to finish (default true)."),
                "until": { "type": "string", "enum": ["committed", "dom", "settled"],
                    "description": "How far the load must progress before returning (default settled). dom = DOMContentLoaded, much earlier on real pages and enough for snapshot/eval." },
                "timeout_ms": prop("integer", "Wait timeout in ms."),
            }),
            &["url"],
        ),
        tool(
            "check",
            "One-call verification pass: open a headless window, load the url, \
             wait for it, optionally eval JS and screenshot, then close the \
             window. Returns url, title, eval result, shot path, console \
             errors, and timings in one reply. Prefer this over separate \
             open/wait_load/eval/screenshot/close calls for one-shot checks.",
            json!({
                "url": prop("string", "URL to load (https:// implied)."),
                "eval": prop("string", "JS to run once loaded (expression or function body)."),
                "shot": prop("boolean", "Take a screenshot to a temp file."),
                "shot_path": prop("string", "Screenshot destination (implies shot)."),
                "full": prop("boolean", "Screenshot the full document, not just the viewport."),
                "until": { "type": "string", "enum": ["committed", "dom", "settled"],
                    "description": "Load stage gating eval/shot (default settled)." },
                "keep": prop("boolean", "Keep the window open and return its id."),
                "timeout_ms": prop("integer", "Deadline for the whole pass in ms."),
            }),
            &["url"],
        ),
        tool(
            "snapshot",
            "Token-cheap page state as JSON: url, title, visible text (bounded), \
             scroll position, and indexed interactable elements (links, buttons, \
             inputs). Use the returned `ref` indices with click/type_text. \
             Prefer this over screenshot for 'what is on this page'.",
            json!({ "id": prop("integer", ID_DESC) }),
            &[],
        ),
        tool(
            "click",
            "Click an element by CSS selector (disambiguate with nth/contains) or \
             by `ref` from the last snapshot. Dispatches real pointer events and \
             reports what was hit.",
            json!({
                "id": prop("integer", ID_DESC),
                "selector": prop("string", "CSS selector."),
                "nth": prop("integer", "0-based index among selector matches."),
                "contains": prop("string", "Keep only matches whose text contains this."),
                "ref": prop("integer", "Interactable index from the last snapshot."),
            }),
            &[],
        ),
        tool(
            "type_text",
            "Type into an input/textarea/select/contenteditable, targeted like \
             click. Uses native setters + input/change events so React-style \
             controlled inputs see it.",
            json!({
                "id": prop("integer", ID_DESC),
                "selector": prop("string", "CSS selector."),
                "nth": prop("integer", "0-based index among selector matches."),
                "contains": prop("string", "Keep only matches whose text contains this."),
                "ref": prop("integer", "Interactable index from the last snapshot."),
                "text": prop("string", "Text to type. For <select>, the option to pick."),
                "clear": prop("boolean", "Replace the current value (default true; false appends)."),
                "enter": prop("boolean", "Press Enter afterwards (submits the form if unhandled)."),
            }),
            &["text"],
        ),
        tool(
            "eval",
            "Run JavaScript in the page. Accepts an expression ('document.title') \
             or a function body ('const n = 1; return n'); `await` works and a \
             returned Promise is awaited. Result comes back as JSON.",
            json!({
                "id": prop("integer", ID_DESC),
                "js": prop("string", "JavaScript expression or function body."),
                "timeout_ms": prop("integer", "Eval timeout in ms (default 15000)."),
            }),
            &["js"],
        ),
        tool(
            "console",
            "Read the window's capture buffer: console.* output, uncaught \
             exceptions, unhandled rejections, failed resource loads, and HTTP \
             >=400 responses. Answers 'why is the page broken' without pixels.",
            json!({
                "id": prop("integer", ID_DESC),
                "clear": prop("boolean", "Drain what was read, so the next read is a clean diff."),
                "limit": prop("integer", "Return at most the last N entries."),
            }),
            &[],
        ),
        tool(
            "screenshot",
            "Capture the page as a PNG file and return its path. Use `full` for \
             the whole document instead of the viewport. For text/DOM checks, \
             snapshot is much cheaper.",
            json!({
                "id": prop("integer", ID_DESC),
                "path": prop("string", "Output path (default: a temp file)."),
                "full": prop("boolean", "Capture the entire document, not just the viewport."),
            }),
            &[],
        ),
        tool(
            "scroll",
            "Scroll and report where the page landed (no screenshot needed to \
             confirm). Give exactly one of: selector (scrolled into view), to_y \
             (absolute px), by_pages (viewport heights, negative = up; default 1).",
            json!({
                "id": prop("integer", ID_DESC),
                "selector": prop("string", "CSS selector to scroll into view (centered)."),
                "nth": prop("integer", "0-based index among selector matches."),
                "contains": prop("string", "Keep only matches whose text contains this."),
                "to_y": prop("number", "Absolute scroll target in CSS pixels."),
                "by_pages": prop("number", "Relative scroll in viewport heights."),
            }),
            &[],
        ),
        tool(
            "wait_load",
            "Block until the window's load reaches a stage (default: fully \
             settled). until=dom releases at DOMContentLoaded, much earlier \
             on real pages and enough for snapshot/eval checks.",
            json!({
                "id": prop("integer", ID_DESC),
                "until": { "type": "string", "enum": ["committed", "dom", "settled"],
                    "description": "Load stage to wait for (default settled)." },
                "timeout_ms": prop("integer", "Timeout in ms."),
            }),
            &[],
        ),
        tool(
            "upload",
            "Set a file input's files from a path on disk (the standard \
             automation technique; the OS picker never opens).",
            json!({
                "id": prop("integer", ID_DESC),
                "selector": prop("string", "CSS selector of the <input type=file>."),
                "path": prop("string", "File to attach."),
            }),
            &["selector", "path"],
        ),
        tool(
            "challenge",
            "Detect CAPTCHA / anti-bot challenge UI, as structured JSON. With \
             wait=true, polls until it clears or timeout. Detection and hand-off \
             only: if manual_required, call focus to show the window to the \
             human, ask them to solve it, then challenge again with wait=true.",
            json!({
                "id": prop("integer", ID_DESC),
                "wait": prop("boolean", "Poll until the challenge disappears or timeout."),
                "timeout_ms": prop("integer", "Wait timeout in ms."),
            }),
            &[],
        ),
        tool(
            "expect",
            "Assert page state, polling until it holds or timeout (default \
             5000 ms): the selector matches an element, optionally with text \
             containing `text`; `absent` asserts no match instead. On failure \
             the error names what WAS found (match count, actual text), so no \
             follow-up snapshot is needed. The one-call verify primitive.",
            json!({
                "id": prop("integer", ID_DESC),
                "selector": prop("string", "CSS selector to assert on."),
                "nth": prop("integer", "0-based index among selector matches."),
                "contains": prop("string", "Keep only matches whose text contains this (a filter)."),
                "text": prop("string", "Require the matched element's text to contain this (an assertion)."),
                "absent": prop("boolean", "Assert the selector matches nothing."),
                "timeout_ms": prop("integer", "Poll deadline in ms (default 5000; 0 = single check)."),
            }),
            &["selector"],
        ),
        tool(
            "motion",
            "Extract the page's motion spec as JSON: every CSS animation, \
             transition, and Web-Animations-API animation (keyframes, \
             durations, delays, easings), plus @keyframes rules from CSSOM. \
             With `observe`, also samples the live page under virtual time \
             and fits models to script-driven motion (rAF marquees, JS \
             tickers): velocity px/s, loop period, easing, fit r2. \
             Motion as numbers instead of eyeballed frames.",
            json!({
                "id": prop("integer", ID_DESC),
                "observe": prop("boolean", "Also observe and model script-driven motion."),
                "observe_ms": prop("integer", "Observation window in virtual ms (default 2500)."),
            }),
            &[],
        ),
        tool(
            "seek",
            "Freeze animation time for deterministic screenshots: pause every \
             animation and set its currentTime. Give `time_ms` (absolute) or \
             `progress` (0-1, proportional per animation); `resume` unpauses.",
            json!({
                "id": prop("integer", ID_DESC),
                "time_ms": prop("number", "Absolute animation time in ms."),
                "progress": prop("number", "Fractional progress 0.0-1.0 per animation."),
                "resume": prop("boolean", "Unpause all animations instead."),
            }),
            &[],
        ),
        tool(
            "clock",
            "Control the page's virtual clock. Unlike seek (CSS/WAAPI only), \
             this also freezes requestAnimationFrame, performance.now, \
             Date.now, new Date(), Date(), and timers, so script-driven motion (rAF marquees, \
             carousels) becomes deterministic. `pause` freezes, `step` \
             advances by `ms` virtual milliseconds, `set` steps to absolute \
             virtual time `ms`, `resume` returns to real time, `status` \
             reports state (including any Math.random seed). `seed` replaces \
             Math.random with a deterministic PRNG seeded from `seed`, for \
             cross-load reproducibility. Two screenshots at the same virtual \
             time are byte-identical.",
            json!({
                "id": prop("integer", ID_DESC),
                "action": prop("string", "pause | resume | step | set | seed | status"),
                "ms": prop("number", "Milliseconds: amount for step, absolute virtual time for set."),
                "seed": prop("integer", "PRNG seed for the seed action."),
            }),
            &["action"],
        ),
        tool(
            "diff",
            "Perceptual pixel diff of a window against another window or a \
             baseline PNG. Returns match percent and bounding boxes of the \
             largest mismatched regions; optionally writes a heatmap PNG. The \
             numeric score is a convergence signal for pixel-perfect work.",
            json!({
                "id": prop("integer", "First window id."),
                "other": prop("integer", "Second window id to compare against."),
                "baseline": prop("string", "Baseline PNG path (instead of `other`)."),
                "tolerance": prop("integer", "Per-channel tolerance 0-255 before a pixel counts as different (default 8)."),
                "heatmap": prop("string", "Write a mismatch heatmap PNG to this path."),
                "full": prop("boolean", "Diff the full document instead of the viewport."),
            }),
            &["id"],
        ),
        tool(
            "focus",
            "Raise and focus a window, promoting headless/background windows to \
             normal: the human sees the agent's live session, cookies and all.",
            json!({ "id": prop("integer", "Window id to show.") }),
            &["id"],
        ),
        tool(
            "close",
            "Close a window. The daemon and engine stay warm.",
            json!({ "id": prop("integer", "Window id to close.") }),
            &["id"],
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_defaults_to_headless() {
        let req = build_request("open", &json!({ "url": "localhost:3000" })).unwrap();
        let Request::Open { url, mode, .. } = req else {
            panic!("expected Open");
        };
        assert_eq!(url.as_deref(), Some("localhost:3000"));
        assert_eq!(mode, OpenMode::Headless);
    }

    #[test]
    fn open_mode_parses_and_rejects_junk() {
        let Request::Open { mode, .. } =
            build_request("open", &json!({ "mode": "background" })).unwrap()
        else {
            panic!("expected Open");
        };
        assert_eq!(mode, OpenMode::Background);
        assert!(build_request("open", &json!({ "mode": "visible" })).is_err());
    }

    #[test]
    fn goto_defaults_wait_true() {
        let Request::Navigate { url, wait, .. } =
            build_request("goto", &json!({ "url": "example.com" })).unwrap()
        else {
            panic!("expected Navigate");
        };
        assert_eq!(url, "example.com");
        assert!(wait);
        assert!(build_request("goto", &json!({})).is_err());
    }

    #[test]
    fn click_requires_selector_or_ref() {
        assert!(build_request("click", &json!({})).is_err());
        let Request::Click { r#ref, .. } = build_request("click", &json!({ "ref": 4 })).unwrap()
        else {
            panic!("expected Click");
        };
        assert_eq!(r#ref, Some(4));
        let Request::Click {
            selector,
            nth,
            contains,
            ..
        } = build_request(
            "click",
            &json!({ "selector": "a", "nth": 2, "contains": "Docs" }),
        )
        .unwrap()
        else {
            panic!("expected Click");
        };
        assert_eq!(selector.as_deref(), Some("a"));
        assert_eq!(nth, Some(2));
        assert_eq!(contains.as_deref(), Some("Docs"));
    }

    #[test]
    fn type_text_defaults_clear_true() {
        let Request::Type {
            text, clear, enter, ..
        } = build_request("type_text", &json!({ "ref": 1, "text": "hello" })).unwrap()
        else {
            panic!("expected Type");
        };
        assert_eq!(text, "hello");
        assert!(clear);
        assert!(!enter);
        // Needs a target and text.
        assert!(build_request("type_text", &json!({ "text": "x" })).is_err());
        assert!(build_request("type_text", &json!({ "ref": 1 })).is_err());
    }

    #[test]
    fn close_and_focus_require_id() {
        assert!(build_request("close", &json!({})).is_err());
        assert!(matches!(
            build_request("close", &json!({ "id": 3 })),
            Ok(Request::Close { id: 3 })
        ));
        assert!(build_request("focus", &json!({})).is_err());
        assert!(matches!(
            build_request("focus", &json!({ "id": 3 })),
            Ok(Request::Focus { id: 3 })
        ));
    }

    #[test]
    fn unknown_tool_is_an_error() {
        assert!(build_request("teleport", &json!({})).is_err());
    }

    #[test]
    fn seek_requires_a_target_time() {
        assert!(build_request("seek", &json!({})).is_err());
        assert!(matches!(
            build_request("seek", &json!({ "resume": true })),
            Ok(Request::Seek { resume: true, .. })
        ));
        let Request::Seek { progress, .. } =
            build_request("seek", &json!({ "progress": 0.25 })).unwrap()
        else {
            panic!("expected Seek");
        };
        assert_eq!(progress, Some(0.25));
    }

    #[test]
    fn diff_requires_id_and_a_comparand() {
        assert!(build_request("diff", &json!({ "id": 1 })).is_err());
        assert!(build_request("diff", &json!({ "other": 2 })).is_err());
        let Request::Diff {
            id,
            other,
            tolerance,
            ..
        } = build_request("diff", &json!({ "id": 1, "other": 2, "tolerance": 400 })).unwrap()
        else {
            panic!("expected Diff");
        };
        assert_eq!((id, other), (1, Some(2)));
        assert_eq!(tolerance, Some(255)); // clamped
    }

    #[test]
    fn every_tool_definition_maps_to_a_request() {
        // Guard against schema/dispatch drift: every advertised tool
        // must be buildable with minimal valid arguments.
        let minimal: Value = json!({
            "open": {},
            "list_windows": {},
            "goto": { "url": "example.com" },
            "check": { "url": "example.com" },
            "snapshot": {},
            "expect": { "selector": "h1" },
            "click": { "ref": 0 },
            "type_text": { "ref": 0, "text": "x" },
            "eval": { "js": "1+1" },
            "console": {},
            "screenshot": {},
            "scroll": {},
            "wait_load": {},
            "upload": { "selector": "input", "path": "/tmp/x" },
            "challenge": {},
            "motion": {},
            "seek": { "progress": 0.5 },
            "clock": { "action": "pause" },
            "diff": { "id": 1, "other": 2 },
            "focus": { "id": 1 },
            "close": { "id": 1 },
        });
        for def in tool_definitions() {
            let name = def["name"].as_str().unwrap();
            let args = minimal
                .get(name)
                .unwrap_or_else(|| panic!("no minimal args for advertised tool {name}"));
            build_request(name, args)
                .unwrap_or_else(|e| panic!("tool {name} failed to build: {e}"));
        }
    }

    #[test]
    fn tool_schemas_are_well_formed() {
        for def in tool_definitions() {
            let name = def["name"].as_str().expect("tool has a name");
            assert!(
                def["description"].as_str().is_some_and(|d| !d.is_empty()),
                "{name} has a description"
            );
            let schema = &def["inputSchema"];
            assert_eq!(schema["type"], "object", "{name} schema is an object");
            let props = schema["properties"].as_object().unwrap();
            for req in schema["required"].as_array().unwrap() {
                let req = req.as_str().unwrap();
                assert!(
                    props.contains_key(req),
                    "{name}: required {req} missing from properties"
                );
            }
        }
    }

    #[test]
    fn initialize_echoes_protocol_version() {
        let res = initialize_result(&json!({ "protocolVersion": "2025-03-26" }));
        assert_eq!(res["protocolVersion"], "2025-03-26");
        assert_eq!(res["serverInfo"]["name"], "hwatu");
        let res = initialize_result(&json!({}));
        assert_eq!(res["protocolVersion"], PROTOCOL_VERSION);
    }
}
