// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Justin Hong
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

use hwatu_ipc::{ClockAction, LoadStage, OpenMode, Request, Response, Viewport};
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};

static EVENT_THREAD: AtomicBool = AtomicBool::new(false);

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
        if crate::verify_job::cancellation_requested() {
            return 143;
        }
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
    if name == "subscribe_events" {
        return subscribe_events(&args);
    }
    if name == "verify_ui" {
        return crate::verify_job::run_mcp(&args);
    }
    let mut request = build_request(name, &args)?;
    crate::normalize_request_paths(&mut request);
    let response = transact(&request)?;
    match response {
        Response::Err { message, stop } => Err(Response::error_text(&message, &stop)),
        ok => serde_json::to_string(&ok).map_err(|e| e.to_string()),
    }
}

/// Start (once) a background thread that holds a subscribed
/// connection and forwards daemon events to the MCP client as
/// JSON-RPC notifications (`notifications/hwatu/event`). Streaming has
/// no place in MCP's request/response tools, so push IPC maps to the
/// protocol's notification channel instead.
fn subscribe_events(args: &Value) -> Result<String, String> {
    if EVENT_THREAD.swap(true, Ordering::SeqCst) {
        return Ok("{\"subscribed\":true,\"note\":\"already streaming\"}".to_string());
    }
    let kinds = args.get("kinds").and_then(Value::as_array).map(|a| {
        a.iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect::<Vec<_>>()
    });
    let window = opt_u64(args, "window");
    let mut stream =
        crate::connect_or_spawn().map_err(|e| format!("cannot reach hwatu daemon: {e}"))?;
    let request = Request::Subscribe { kinds, window };
    crate::write_request(&mut stream, &request).map_err(|e| format!("write failed: {e}"))?;
    std::thread::spawn(move || {
        while let Ok(Some(line)) = crate::read_text_frame(&mut stream) {
            let Ok(event) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let note = json!({
                "jsonrpc": "2.0",
                "method": "notifications/hwatu/event",
                "params": event,
            });
            send(&std::io::stdout(), &note);
        }
        EVENT_THREAD.store(false, Ordering::SeqCst);
    });
    Ok("{\"subscribed\":true}".to_string())
}

/// One request per connection, like the CLI.
fn transact(request: &Request) -> Result<Response, String> {
    crate::transact_request(request).map_err(|e| format!("daemon request failed: {e}"))
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

/// Vision-table command names accepted as MCP tool names, mapped to the
/// original tool names. Both spellings work; tools/list advertises the
/// vision names.
fn canonical_tool(name: &str) -> &str {
    match name {
        "shot" => "screenshot",
        "type" => "type_text",
        "fill" => "fill_form",
        "batch" => "batch_actions",
        "content" => "get_content",
        "show" => "focus",
        "hide" => "unfocus",
        other => other,
    }
}

/// Map an MCP tool call onto the wire [`Request`].
pub(crate) fn build_request(name: &str, args: &Value) -> Result<Request, String> {
    let name = canonical_tool(name);
    let id = opt_u64(args, "id");
    let timeout_ms = opt_u64(args, "timeout_ms");
    match name {
        "open" => {
            // Windows always open headless; `show` is the only way to make
            // one visible.
            let mode = match opt_str(args, "mode").as_deref() {
                None | Some("headless") => OpenMode::Headless,
                Some(other) => {
                    return Err(format!(
                        "invalid mode {other:?}: windows always open headless; use show to make one visible"
                    ))
                }
            };
            Ok(Request::Open {
                url: opt_str(args, "url"),
                app_id: opt_str(args, "app_id"),
                mode,
                profile: opt_str(args, "profile"),
            })
        }
        "list_windows" => Ok(Request::List),
        "close" => Ok(Request::Close {
            id: opt_u64(args, "id").ok_or("missing required argument: id")?,
        }),
        "press" => {
            let key = req_str(args, "key")?;
            Ok(Request::Press {
                id,
                key: hwatu_ipc::PressKey::parse(&key).ok_or_else(|| {
                    format!("invalid key {key:?} (want Tab|Enter|Escape|ArrowLeft|ArrowRight)")
                })?,
                timeout_ms,
            })
        }
        "focus" => Ok(Request::Focus {
            id: opt_u64(args, "id").ok_or("missing required argument: id")?,
        }),
        "unfocus" => Ok(Request::Unfocus {
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
            data: false,
        }),
        "wait_load" => Ok(Request::WaitLoad {
            id,
            until: parse_until(args)?,
            timeout_ms,
        }),
        "batch_actions" => {
            let entries = args
                .get("actions")
                .and_then(Value::as_array)
                .ok_or("batch_actions needs `actions` array")?;
            let mut actions = Vec::with_capacity(entries.len());
            for (index, entry) in entries.iter().enumerate() {
                let tool = entry
                    .get("tool")
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("actions[{index}] needs string `tool`"))?;
                let empty = json!({});
                let step_args = entry.get("args").unwrap_or(&empty);
                let action = build_request(tool, step_args)
                    .map_err(|e| format!("actions[{index}] {tool}: {e}"))?;
                actions.push(action);
            }
            Request::validate_batch(&actions).map_err(|e| format!("batch_actions: {e}"))?;
            Ok(Request::Batch { actions })
        }
        "check" => {
            let viewports = parse_viewports(args)?;
            let baseline_dir = opt_str(args, "baseline_dir");
            if baseline_dir.is_some() && viewports.is_empty() {
                return Err("baseline_dir needs viewports".into());
            }
            Ok(Request::Check {
                url: Some(req_str(args, "url")?),
                render: None,
                base: None,
                eval: opt_str(args, "eval"),
                shot: opt_bool(args, "shot").unwrap_or(false),
                shot_path: opt_str(args, "shot_path"),
                shot_data: false,
                full: opt_bool(args, "full").unwrap_or(false),
                baseline: opt_str(args, "baseline"),
                baseline_data: None,
                tolerance: opt_u64(args, "tolerance").map(|v| v.min(255) as u8),
                min_region_px: opt_u64(args, "min_region_px")
                    .map(|v| v.min(u32::MAX as u64) as u32),
                heatmap: opt_str(args, "heatmap"),
                heatmap_data: false,
                until: parse_until(args)?,
                keep: opt_bool(args, "keep").unwrap_or(false),
                timeout_ms,
                viewports,
                baseline_dir,
            })
        }
        "render" => {
            let html = req_str(args, "html")?;
            if html.len() > hwatu_ipc::RENDER_MAX_BYTES {
                return Err(format!(
                    "html is {} bytes; the cap is {}",
                    html.len(),
                    hwatu_ipc::RENDER_MAX_BYTES
                ));
            }
            let viewports = parse_viewports(args)?;
            let baseline_dir = opt_str(args, "baseline_dir");
            if baseline_dir.is_some() && viewports.is_empty() {
                return Err("baseline_dir needs viewports".into());
            }
            Ok(Request::Check {
                url: None,
                render: Some(html),
                base: opt_str(args, "base"),
                eval: opt_str(args, "eval"),
                shot: opt_bool(args, "shot").unwrap_or(false),
                shot_path: opt_str(args, "shot_path"),
                shot_data: false,
                full: opt_bool(args, "full").unwrap_or(false),
                baseline: opt_str(args, "baseline"),
                baseline_data: None,
                tolerance: opt_u64(args, "tolerance").map(|v| v.min(255) as u8),
                min_region_px: opt_u64(args, "min_region_px")
                    .map(|v| v.min(u32::MAX as u64) as u32),
                heatmap: opt_str(args, "heatmap"),
                heatmap_data: false,
                until: parse_until(args)?,
                keep: opt_bool(args, "keep").unwrap_or(false),
                timeout_ms,
                viewports,
                baseline_dir,
            })
        }
        "prefetch" => Ok(Request::Prefetch {
            url: req_str(args, "url")?,
        }),
        "snapshot" => Ok(Request::Snapshot {
            id,
            diff: opt_bool(args, "diff").unwrap_or(false),
            rect: opt_bool(args, "rect").unwrap_or(false),
            budget: opt_u64(args, "budget").map(|n| n as usize),
            timeout_ms,
        }),
        "expect" => Ok(Request::Expect {
            id,
            selector: req_str(args, "selector")?,
            nth: opt_u32(args, "nth"),
            contains: opt_str(args, "contains"),
            text: opt_str(args, "text"),
            absent: opt_bool(args, "absent").unwrap_or(false),
            visible: opt_bool(args, "visible").unwrap_or(false),
            timeout_ms,
            watch: false,
        }),
        "click" => {
            let selector = opt_str(args, "selector");
            let r#ref = opt_u32(args, "ref");
            let at = match (
                args.get("x").and_then(Value::as_f64),
                args.get("y").and_then(Value::as_f64),
            ) {
                (Some(x), Some(y)) => Some([x, y]),
                (None, None) => None,
                _ => return Err("click at a point needs both `x` and `y`".into()),
            };
            let targets = usize::from(selector.is_some())
                + usize::from(r#ref.is_some())
                + usize::from(at.is_some());
            if targets != 1 {
                return Err("click needs exactly one of `selector`, `ref`, or `x`+`y`".into());
            }
            let trusted = opt_bool(args, "trusted").unwrap_or(false);
            if at.is_some() && !trusted {
                return Err("click at `x`,`y` needs `trusted: true`".into());
            }
            Ok(Request::Click {
                id,
                selector,
                nth: opt_u32(args, "nth"),
                contains: opt_str(args, "contains"),
                r#ref,
                at,
                trusted: opt_bool(args, "trusted").unwrap_or(false),
                confirm: opt_bool(args, "confirm").unwrap_or(false),
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
                trusted: opt_bool(args, "trusted").unwrap_or(false),
                clear: opt_bool(args, "clear").unwrap_or(true),
                enter: opt_bool(args, "enter").unwrap_or(false),
                timeout_ms,
            })
        }
        "paste" => {
            let selector = opt_str(args, "selector");
            let r#ref = opt_u32(args, "ref");
            if selector.is_none() && r#ref.is_none() {
                return Err("paste needs `selector` or `ref`".into());
            }
            Ok(Request::Paste {
                id,
                selector,
                nth: opt_u32(args, "nth"),
                contains: opt_str(args, "contains"),
                r#ref,
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
        "net" => Ok(Request::Net {
            id,
            clear: opt_bool(args, "clear").unwrap_or(false),
            limit: opt_u64(args, "limit").map(|v| v as usize),
        }),
        "upload" => Ok(Request::Upload {
            id,
            selector: req_str(args, "selector")?,
            path: req_str(args, "path")?,
            data: None,
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
                baseline_data: None,
                tolerance: opt_u64(args, "tolerance").map(|v| v.min(255) as u8),
                min_region_px: opt_u64(args, "min_region_px")
                    .map(|v| v.min(u32::MAX as u64) as u32),
                heatmap: opt_str(args, "heatmap"),
                heatmap_data: false,
                full: opt_bool(args, "full").unwrap_or(false),
                timeout_ms,
            })
        }
        "subscribe_events" => {
            // Streaming is intercepted in handle_tool_call; this arm
            // exists so the tool-to-request mapping stays total (the
            // minimal-args test walks it).
            Ok(Request::Subscribe {
                kinds: args.get("kinds").and_then(Value::as_array).map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                }),
                window: opt_u64(args, "window"),
            })
        }
        "get_content" => Ok(Request::GetContent {
            id,
            format: match opt_str(args, "format") {
                None => hwatu_ipc::ContentFormat::default(),
                Some(v) => hwatu_ipc::ContentFormat::parse(&v)
                    .ok_or_else(|| format!("invalid format {v:?} (want text|html|title|links)"))?,
            },
            selector: opt_str(args, "selector"),
            nth: opt_u32(args, "nth"),
            contains: opt_str(args, "contains"),
            max_chars: opt_u64(args, "max_chars").map(|v| v as usize),
            timeout_ms,
        }),
        "fill_form" => {
            let entries = args
                .get("fields")
                .and_then(Value::as_array)
                .ok_or("fill_form needs `fields` array")?;
            let mut fields = Vec::with_capacity(entries.len());
            for (index, entry) in entries.iter().enumerate() {
                let field: hwatu_ipc::FormField = serde_json::from_value(entry.clone())
                    .map_err(|e| format!("fields[{index}]: {e}"))?;
                fields.push(field);
            }
            Ok(Request::FillForm {
                id,
                fields,
                submit: opt_bool(args, "submit").unwrap_or(false),
                timeout_ms,
            })
        }
        "list_downloads" => Ok(Request::ListDownloads {
            limit: opt_u64(args, "limit").map(|v| v as usize),
        }),
        "drop_file" => Ok(Request::DropFile {
            id,
            selector: req_str(args, "selector")?,
            nth: opt_u32(args, "nth"),
            contains: opt_str(args, "contains"),
            path: opt_str(args, "path"),
            data: opt_str(args, "data"),
            name: opt_str(args, "name"),
            mime: opt_str(args, "mime"),
            timeout_ms,
        }),
        "auth_context" => Ok(Request::AuthContext { id }),
        "fill_login" => Ok(Request::FillLogin {
            id,
            kind: match opt_str(args, "kind") {
                None => hwatu_ipc::LoginFill::Password,
                Some(v) => hwatu_ipc::LoginFill::parse(&v)
                    .ok_or_else(|| format!("invalid kind {v:?} (want password|otp)"))?,
            },
            host: opt_str(args, "host"),
            timeout_ms,
        }),
        "fork" => Ok(Request::Fork {
            id,
            name: opt_str(args, "name"),
            count: opt_u32(args, "count"),
            timeout_ms,
        }),
        "list_forks" => Ok(Request::ListForks),
        "list_frames" => Ok(Request::ListFrames { id, timeout_ms }),
        "try_until" => {
            let entries = args
                .get("alternatives")
                .and_then(Value::as_array)
                .ok_or("try_until needs `alternatives` array")?;
            let mut alternatives = Vec::with_capacity(entries.len());
            for (index, entry) in entries.iter().enumerate() {
                let tool = entry
                    .get("tool")
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("alternatives[{index}] needs string `tool`"))?;
                let empty = json!({});
                let step_args = entry.get("args").unwrap_or(&empty);
                let action = build_request(tool, step_args)
                    .map_err(|e| format!("alternatives[{index}] {tool}: {e}"))?;
                alternatives.push(action);
            }
            Request::validate_try_until(&alternatives).map_err(|e| format!("try_until: {e}"))?;
            Ok(Request::TryUntil {
                id,
                alternatives,
                timeout_ms,
            })
        }
        "scout" => Ok(Request::Scout {
            url: req_str(args, "url")?,
            depth: opt_u32(args, "depth"),
            max_pages: opt_u32(args, "max_pages"),
            filter: opt_str(args, "filter"),
            budget: opt_u64(args, "budget").map(|v| v as usize),
            profile: opt_str(args, "profile"),
            timeout_ms,
        }),
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

/// Optional `viewports` argument: a `"360x640,1920x1080"` string (the
/// CLI spelling, friendly to LLM callers). Absent means no sweep.
fn parse_viewports(args: &Value) -> Result<Vec<Viewport>, String> {
    match opt_str(args, "viewports") {
        None => Ok(Vec::new()),
        Some(v) => Viewport::parse_list(&v),
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
            "Open a headless window and return its id. Use show to make it \
             visible to the user.",
            json!({
                "url": prop("string", "URL to load (https:// implied). Omit for about:blank."),
                "app_id": prop("string", "Optional label reported by list_windows."),
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
            "verify_ui",
            "Run one harness-neutral verification job from a versioned JSON spec: optional preflight, optional dev-server lifecycle with readiness polling, one responsive Hwatu check, screenshots, deterministic assertions, source-staleness detection, cleanup, and an evidence report. The same spec works with `hwatu verify` from any CLI harness.",
            json!({
                "spec_path": prop("string", "Path to a versioned Hwatu verification job JSON file."),
                "spec": {
                    "type": "object",
                    "description": "Inline browser-only verification job object. Process commands and cwd/output-path overrides are rejected; source paths and symlinks cannot escape the MCP working directory. Evidence uses a runtime-scoped Hwatu directory. Use a reviewed spec_path for process commands. Use this or spec_path, not both."
                },
            }),
            &[],
        ),
        tool(
            "check",
            "One-call verification pass: open a headless window, load the url, \
             wait for it, optionally eval JS, screenshot, and pixel-diff against \
             a baseline PNG, then close the window. Returns url, title, eval \
             result, shot path, diff score/regions, console errors, and timings \
             in one reply. Prefer this over separate \
             open/wait_load/eval/screenshot/close calls for one-shot checks.",
            json!({
                "url": prop("string", "URL to load (https:// implied)."),
                "eval": prop("string", "JS to run once loaded (expression or function body)."),
                "shot": prop("boolean", "Take a screenshot to a temp file."),
                "shot_path": prop("string", "Screenshot destination (implies shot)."),
                "full": prop("boolean", "Screenshot/diff the full document, not just the viewport."),
                "baseline": prop("string", "Baseline PNG path: pixel-diff the loaded page against it and include match_percent + mismatch regions in the reply."),
                "tolerance": prop("integer", "Per-channel diff tolerance 0-255 (default 8; only with baseline)."),
                "min_region_px": prop("integer", "Region significance floor in mismatched pixels (default 1024; only with baseline). Gate on the reply's significant_regions == 0 rather than match_percent: the mean is diluted by unchanged background."),
                "heatmap": prop("string", "Write a mismatch heatmap PNG here (only with baseline)."),
                "viewports": prop("string", "Multi-viewport sweep: comma-separated sizes (e.g. \"360x640,768x1024,1920x1080\"). Runs the same pass at each size on one window and returns per-viewport results under `viewports` (size, load_ms, eval, shot, diff). Screenshot paths get a -WxH suffix per size."),
                "baseline_dir": prop("string", "Directory of per-size baseline PNGs (<dir>/360x640.png) for the viewport sweep; each size diffs against its own file. Only with viewports."),
                "until": { "type": "string", "enum": ["committed", "dom", "settled"],
                    "description": "Load stage gating eval/shot (default settled)." },
                "keep": prop("boolean", "Keep the window open and return its id."),
                "timeout_ms": prop("integer", "Deadline for the whole pass in ms."),
            }),
            &["url"],
        ),
        tool(
            "render",
            "Render generated HTML directly, no server or temp file needed: load \
             the markup in a headless window, wait for it, optionally eval JS, \
             screenshot, and pixel-diff against a baseline, then close the \
             window. Same one-call pass as check, but the input is markup in \
             hand instead of a deployed URL. Use `base` so relative asset paths \
             (img/css/js) resolve.",
            json!({
                "html": prop("string", "The HTML document to render."),
                "base": prop("string", "Base URL for resolving relative references in the markup."),
                "eval": prop("string", "JS to run once loaded (expression or function body)."),
                "shot": prop("boolean", "Take a screenshot to a temp file."),
                "shot_path": prop("string", "Screenshot destination (implies shot)."),
                "full": prop("boolean", "Screenshot/diff the full document, not just the viewport."),
                "baseline": prop("string", "Baseline PNG path: pixel-diff the rendered page against it."),
                "tolerance": prop("integer", "Per-channel diff tolerance 0-255 (default 8; only with baseline)."),
                "min_region_px": prop("integer", "Region significance floor in mismatched pixels (default 1024; only with baseline). Gate on the reply's significant_regions == 0 rather than match_percent: the mean is diluted by unchanged background."),
                "heatmap": prop("string", "Write a mismatch heatmap PNG here (only with baseline)."),
                "viewports": prop("string", "Multi-viewport sweep: comma-separated sizes (e.g. \"360x640,1920x1080\"); per-viewport results under `viewports`."),
                "baseline_dir": prop("string", "Directory of per-size baseline PNGs for the sweep (only with viewports)."),
                "until": { "type": "string", "enum": ["committed", "dom", "settled"],
                    "description": "Load stage gating eval/shot (default settled)." },
                "keep": prop("boolean", "Keep the window open and return its id."),
                "timeout_ms": prop("integer", "Deadline for the whole pass in ms."),
            }),
            &["html"],
        ),
        tool(
            "prefetch",
            "Speculatively start loading a url in a headless window and return \
             immediately. The next `check` of the same url adopts the warm \
             window, paying ~0 load latency. Call it right after editing code \
             (while the dev server rebuilds and you compose the check). \
             Fire-and-forget: unclaimed prefetches expire after ~30s.",
            json!({
                "url": prop("string", "URL to start loading (https:// implied)."),
            }),
            &["url"],
        ),
        tool(
            "snapshot",
            "Token-cheap page state as JSON: url, title, visible text (bounded), \
             scroll position, and indexed interactable elements (links, buttons, \
             inputs). Use the returned `ref` indices with click/type_text. \
             Prefer this over screenshot for 'what is on this page'. With \
             diff=true, returns only what changed since the last diff snapshot \
             of the window ({added, removed, changed, unchanged_count}); the \
             first diff (and the first after a navigation) returns the full \
             snapshot with baseline_established=true.",
            json!({
                "id": prop("integer", ID_DESC),
                "diff": prop("boolean", "Return only changes since the last diff \
                     snapshot of this window instead of the full page state."),
                "rect": prop("boolean", "Include viewport CSS rectangles as [x, y, width, height] for interactables."),
            }),
            &[],
        ),
        tool(
            "click",
            "Click an element by CSS selector (disambiguate with nth/contains), \
             by `ref` from the last snapshot, or at a point x,y (trusted only). \
             Dispatches JS pointer events by default; trusted=true delivers \
             native input, including into cross-origin frames such as CAPTCHA \
             widgets. ping reports capabilities.trusted_input_headless: where it \
             is false, trusted input needs a shown window.",
            json!({
                "id": prop("integer", ID_DESC),
                "selector": prop("string", "CSS selector."),
                "nth": prop("integer", "0-based index among selector matches."),
                "contains": prop("string", "Keep only matches whose text contains this."),
                "ref": prop("integer", "Interactable index from the last snapshot."),
                "x": prop("number", "Point click: top-viewport CSS x (shot pixel / devicePixelRatio). Needs y and trusted."),
                "y": prop("number", "Point click: top-viewport CSS y. Needs x and trusted."),
                "trusted": prop("boolean", "Request native trusted input synthesis instead of JS events."),
                "confirm": prop("boolean", "The user confirmed this click in the current session. Without it, a click on a payment, send, or delete control returns stop.reason = irreversible_action and is not performed."),
            }),
            &[],
        ),
        tool(
            "type",
            "Type into an input/textarea/select/contenteditable, targeted like \
             click. Uses native setters + input/change events by default; \
             trusted=true requests native trusted key input synthesis. Trusted \
             input is refused for background/headless windows because compositor \
             delivery would visibly focus them.",
            json!({
                "id": prop("integer", ID_DESC),
                "selector": prop("string", "CSS selector."),
                "nth": prop("integer", "0-based index among selector matches."),
                "contains": prop("string", "Keep only matches whose text contains this."),
                "ref": prop("integer", "Interactable index from the last snapshot."),
                "text": prop("string", "Text to type. For <select>, the option to pick."),
                "trusted": prop("boolean", "Request native trusted key input synthesis instead of JS events."),
                "clear": prop("boolean", "Replace the current value (default true; false appends)."),
                "enter": prop("boolean", "Press Enter afterwards (submits the form if unhandled)."),
            }),
            &["text"],
        ),
        tool(
            "paste",
            "Paste from the compositor clipboard into an element targeted like click. \
             Always uses native trusted input: clicks/focuses the element, then \
             presses Ctrl+V with wtype while the daemon owns compositor focus. \
             Refused for background/headless windows to preserve headless focus \
             isolation.",
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
            "net",
            "Structured per-window network request log: method, final url, HTTP \
             status, resource type (inferred from response MIME), start_ms offset \
             from navigation start, and duration_ms. Bounded 500-entry ring \
             buffer. Assert 'the POST to /api returned 200' instead of reading a \
             success toast.",
            json!({
                "id": prop("integer", ID_DESC),
                "clear": prop("boolean", "Empty the buffer after reading."),
                "limit": prop("integer", "Return at most the last N entries."),
            }),
            &[],
        ),
        tool(
            "shot",
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
            "batch",
            "Run a bounded ordered batch of allowlisted browser actions in one IPC request. The daemon validates every step before executing any, runs sequentially, stops on first failure, and returns per-step ok/error/skipped results with explicit partial-execution semantics.",
            json!({
                "actions": {
                    "type": "array",
                    "maxItems": hwatu_ipc::BATCH_MAX_ACTIONS,
                    "items": {
                        "type": "object",
                        "properties": {
                            "tool": { "type": "string", "enum": ["eval", "goto", "shot", "screenshot", "wait_load", "scroll", "snapshot", "click", "type", "type_text", "press", "console", "expect"],
                                "description": "Allowlisted tool to execute as this batch step." },
                            "args": { "type": "object", "description": "Arguments for that tool, using the same schema as the standalone MCP tool." }
                        },
                        "required": ["tool"]
                    },
                    "description": "Ordered action steps. Nested batch_actions and unsupported tools are rejected before any step runs."
                }
            }),
            &["actions"],
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
                "visible": prop("boolean", "Require the element to be actually visible: nonzero box, not display:none/visibility:hidden/opacity:0 (self or ancestor), scrolled into view when off-screen, and not covered at its center or four corners (multi-point elementFromPoint occlusion check). Failure names the exact point and what covers it."),
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
             baseline PNG. Returns match percent, worst_region, a count of \
             significant_regions, and bounding boxes of the largest mismatched \
             regions; optionally writes a heatmap PNG. Gate on \
             significant_regions == 0 (match_percent is diluted by unchanged \
             background; a moved button still reads 99%+). match_percent is \
             the convergence/trend signal for pixel-perfect work.",
            json!({
                "id": prop("integer", "First window id."),
                "other": prop("integer", "Second window id to compare against."),
                "baseline": prop("string", "Baseline PNG path (instead of `other`)."),
                "tolerance": prop("integer", "Per-channel tolerance 0-255 before a pixel counts as different (default 8)."),
                "min_region_px": prop("integer", "Region significance floor in mismatched pixels (default 1024). Regions holding at least this many mismatched pixels count toward significant_regions."),
                "heatmap": prop("string", "Write a mismatch heatmap PNG to this path."),
                "full": prop("boolean", "Diff the full document instead of the viewport."),
            }),
            &["id"],
        ),
        tool(
            "show",
            "Make a window visible to the user, with its live session, cookies, \
             and page state. The only way a window becomes visible. Use when \
             the user must see or act on the page (a challenge the agent \
             could not clear, a judgment call). Call hide when done. \
             `focus` is a legacy alias.",
            json!({ "id": prop("integer", "Window id to show.") }),
            &["id"],
        ),
        tool(
            "hide",
            "Return a shown window to headless. The live session, cookies, \
             and page state are kept. `unfocus` is a legacy alias.",
            json!({ "id": prop("integer", "Window id to hide.") }),
            &["id"],
        ),
        tool(
            "press",
            "Press a key in the page without showing the window: Tab, Enter, \
             Escape, ArrowLeft, or ArrowRight.",
            json!({
                "id": prop("integer", "Window id (default: last targeted)."),
                "key": prop("string", "Tab | Enter | Escape | ArrowLeft | ArrowRight"),
                "timeout_ms": prop("integer", "Timeout in milliseconds."),
            }),
            &["key"],
        ),
        tool(
            "subscribe_events",
            "Start streaming daemon events (load lifecycle, console captures, \
             downloads, window open/close/focus) to this MCP session as \
             notifications/hwatu/event notifications. Events carry window_id \
             and a per-connection monotonic seq. Idempotent: a second call \
             reports the stream already runs.",
            json!({
                "kinds": { "type": "array", "items": { "type": "string", "enum": ["load", "console", "download", "window"] },
                    "description": "Only these event kinds (default: all)." },
                "window": prop("integer", "Only events for this window id (default: all windows)."),
            }),
            &[],
        ),
        tool(
            "close",
            "Close a window. The daemon and engine stay warm.",
            json!({ "id": prop("integer", "Window id to close.") }),
            &["id"],
        ),
        tool(
            "content",
            "Extract page content in one call: visible text (default), outer \
             HTML, title+url, or de-duplicated links. Optionally scoped to one \
             element by selector. Cheaper than eval for plain extraction.",
            json!({
                "id": prop("integer", ID_DESC),
                "format": { "type": "string", "enum": ["text", "html", "title", "links"],
                    "description": "What to return (default text)." },
                "selector": prop("string", "Scope extraction to this element instead of the document."),
                "nth": prop("integer", "0-based index among selector matches (default 0)."),
                "contains": prop("string", "Keep only selector matches whose text contains this."),
                "max_chars": prop("integer", "Character budget for text/html output (default 65536)."),
            }),
            &[],
        ),
        tool(
            "fill",
            "Fill several form fields in one roundtrip: each field targets an \
             element (selector or snapshot ref) and sets a text value, a \
             select option, or a checkbox/radio state, with framework-safe \
             events. Fields apply in order; the reply reports which landed. \
             With submit, the last field's form is submitted afterwards.",
            json!({
                "id": prop("integer", ID_DESC),
                "fields": { "type": "array", "items": { "type": "object", "properties": {
                    "selector": prop("string", "CSS selector for the field."),
                    "nth": prop("integer", "0-based index among matches."),
                    "contains": prop("string", "Keep only matches whose text contains this."),
                    "ref": prop("integer", "Interactable index from the last snapshot (instead of selector)."),
                    "value": prop("string", "Text to type or select option (value or label)."),
                    "checked": prop("boolean", "Checkbox/radio state (instead of value)."),
                }},
                    "description": "Fields to fill, in order." },
                "submit": prop("boolean", "Submit the last field's form after filling."),
            }),
            &["fields"],
        ),
        tool(
            "list_downloads",
            "List downloads this daemon session started: url, destination, \
             state (active/finished/failed), originating window.",
            json!({
                "limit": prop("integer", "Return at most the last N entries."),
            }),
            &[],
        ),
        tool(
            "drop_file",
            "Simulate dropping a file onto an element (drag-and-drop upload \
             zones without an <input type=file>): dispatches dragenter/\
             dragover/drop with a real DataTransfer, and also feeds any \
             embedded file input.",
            json!({
                "id": prop("integer", ID_DESC),
                "selector": prop("string", "Drop target element."),
                "nth": prop("integer", "0-based index among matches."),
                "contains": prop("string", "Keep only matches whose text contains this."),
                "path": prop("string", "File path readable by the daemon."),
                "name": prop("string", "File name presented to the page (default: basename)."),
                "mime": prop("string", "MIME type presented to the page (default: guessed)."),
            }),
            &["selector", "path"],
        ),
        tool(
            "fill_login",
            "Fill the page's login form from the user's password manager \
             (pass or Bitwarden CLI): kind=password fills username+password, \
             kind=otp fills the one-time-code field from pass-otp / bw totp. \
             Secrets go into the page only; the reply never contains them. \
             The headless answer to 2FA: enroll the agent profile's TOTP \
             secret once, fill codes forever.",
            json!({
                "id": prop("integer", ID_DESC),
                "kind": { "type": "string", "enum": ["password", "otp"],
                    "description": "What to fill (default password)." },
                "host": prop("string", "Override the store-lookup host (default: page host)."),
            }),
            &[],
        ),
        tool(
            "auth_context",
            "Report login/session state for a window's origin: cookie names \
             and counts (values never leave the page), localStorage keys, and \
             a likely_authenticated heuristic. Use before attempting a flow \
             to decide between proceeding and a handoff.",
            json!({ "id": prop("integer", ID_DESC) }),
            &[],
        ),
        tool(
            "fork",
            "Duplicate a live window: new headless window(s) on the same \
             cookie profile navigated to the same URL. Try risky or \
             alternative paths on the fork while the original keeps its \
             state; close forks with close.",
            json!({
                "id": prop("integer", "Source window (defaults like other id-less calls)."),
                "name": prop("string", "Label reported by list_forks."),
                "count": prop("integer", "How many forks (default 1, max 8)."),
            }),
            &[],
        ),
        tool(
            "list_forks",
            "List live forked windows: id, name, parent, url.",
            json!({}),
            &[],
        ),
        tool(
            "list_frames",
            "Enumerate the page's frame tree: index, name, src, rect, and \
             whether each frame is same-origin accessible. Selector-based \
             verbs (click, type_text, get_content, fill_form) automatically \
             reach elements inside accessible frames; cross-origin frames \
             are listed but not scriptable.",
            json!({ "id": prop("integer", ID_DESC) }),
            &[],
        ),
        tool(
            "try_until",
            "Try alternative actions in order until one succeeds: each \
             alternative is a {tool, args} pair (click, type_text, or expect). \
             Returns the winning alternative's index and result, or every \
             failure. The one-call answer to 'the button is one of these \
             selectors'.",
            json!({
                "id": prop("integer", ID_DESC),
                "alternatives": { "type": "array", "items": { "type": "object", "properties": {
                    "tool": prop("string", "click | type_text | expect"),
                    "args": { "type": "object", "description": "That tool's arguments." },
                }},
                    "description": "Candidate actions, tried in order (max 8)." },
                "timeout_ms": prop("integer", "Overall deadline (default 5000)."),
            }),
            &["alternatives"],
        ),
        tool(
            "scout",
            "Bounded same-host crawl: load a URL headlessly, harvest \
             text/title/links, follow same-host links up to depth/max_pages \
             on one warm window, and return per-page results. Reconnaissance \
             in one call, zero focus changes.",
            json!({
                "url": prop("string", "Start URL (https:// implied)."),
                "depth": prop("integer", "Link-following depth (default 1, max 2)."),
                "max_pages": prop("integer", "Total page budget (default 5, max 10)."),
                "filter": prop("string", "Only follow links whose URL or text contains this."),
                "budget": prop("integer", "Character budget per page snapshot (default 2000)."),
                "profile": prop("string", "Cookie/site-data profile for the crawl."),
                "timeout_ms": prop("integer", "Deadline for the whole crawl (default 60000)."),
            }),
            &["url"],
        ),
    ]
}

#[cfg(test)]
mod tests {
    /// An absolute path on every OS.
    fn abs(name: &str) -> String {
        std::env::temp_dir()
            .join(name)
            .to_string_lossy()
            .into_owned()
    }

    use super::*;

    /// The MCP check tool takes `viewports` as the CLI-style size
    /// string and `baseline_dir`; bad sizes and baseline_dir-without-
    /// viewports are argument errors, and a plain check carries an
    /// empty sweep.
    #[test]
    fn check_tool_parses_viewports() {
        let req = build_request(
            "check",
            &json!({
                "url": "localhost:3000",
                "viewports": "360x640,1920x1080",
                "baseline_dir": abs("base"),
            }),
        )
        .unwrap();
        let Request::Check {
            viewports,
            baseline_dir,
            ..
        } = req
        else {
            panic!("expected Check");
        };
        assert_eq!(
            viewports,
            vec![Viewport { w: 360, h: 640 }, Viewport { w: 1920, h: 1080 }]
        );
        assert_eq!(baseline_dir.as_deref(), Some(abs("base").as_str()));

        assert!(build_request("check", &json!({ "url": "x", "viewports": "banana" })).is_err());
        assert!(build_request("check", &json!({ "url": "x", "baseline_dir": abs("b") })).is_err());

        let Request::Check { viewports, .. } =
            build_request("check", &json!({ "url": "x" })).unwrap()
        else {
            panic!("expected Check");
        };
        assert!(viewports.is_empty());
    }

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
    fn open_mode_is_headless_only() {
        let Request::Open { mode, .. } =
            build_request("open", &json!({ "mode": "headless" })).unwrap()
        else {
            panic!("expected Open");
        };
        assert_eq!(mode, OpenMode::Headless);
        for visible in ["background", "normal", "focus", "visible"] {
            assert!(build_request("open", &json!({ "mode": visible })).is_err());
        }
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
            trusted,
            ..
        } = build_request(
            "click",
            &json!({ "selector": "a", "nth": 2, "contains": "Docs", "trusted": true }),
        )
        .unwrap()
        else {
            panic!("expected Click");
        };
        assert_eq!(selector.as_deref(), Some("a"));
        assert_eq!(nth, Some(2));
        assert_eq!(contains.as_deref(), Some("Docs"));
        assert!(trusted);
    }

    #[test]
    fn type_text_defaults_clear_true() {
        let Request::Type {
            text,
            trusted,
            clear,
            enter,
            ..
        } = build_request("type_text", &json!({ "ref": 1, "text": "hello" })).unwrap()
        else {
            panic!("expected Type");
        };
        assert_eq!(text, "hello");
        assert!(!trusted);
        assert!(clear);
        assert!(!enter);
        // Needs a target and text.
        assert!(build_request("type_text", &json!({ "text": "x" })).is_err());
        assert!(build_request("type_text", &json!({ "ref": 1 })).is_err());
    }

    #[test]
    fn paste_requires_selector_or_ref() {
        assert!(build_request("paste", &json!({})).is_err());
        let Request::Paste { r#ref, .. } = build_request("paste", &json!({ "ref": 4 })).unwrap()
        else {
            panic!("expected Paste");
        };
        assert_eq!(r#ref, Some(4));

        let Request::Paste {
            selector,
            nth,
            contains,
            ..
        } = build_request(
            "paste",
            &json!({ "selector": "textarea", "nth": 1, "contains": "Bio" }),
        )
        .unwrap()
        else {
            panic!("expected Paste");
        };
        assert_eq!(selector.as_deref(), Some("textarea"));
        assert_eq!(nth, Some(1));
        assert_eq!(contains.as_deref(), Some("Bio"));
    }

    #[test]
    fn batch_actions_builds_and_rejects_unsupported_steps() {
        let Request::Batch { actions } = build_request(
            "batch_actions",
            &json!({
                "actions": [
                    { "tool": "eval", "args": { "js": "return 1" } },
                    { "tool": "click", "args": { "ref": 0 } },
                    { "tool": "expect", "args": { "selector": ".done", "text": "Saved" } }
                ]
            }),
        )
        .unwrap() else {
            panic!("expected Batch");
        };
        assert_eq!(actions.len(), 3);
        assert!(matches!(actions[1], Request::Click { r#ref: Some(0), .. }));

        // open/close are batch actions now (whole-journey flows); a
        // window-management verb like focus still is not.
        let Request::Batch { actions } = build_request(
            "batch_actions",
            &json!({ "actions": [{ "tool": "close", "args": { "id": 1 } }] }),
        )
        .unwrap() else {
            panic!("expected Batch");
        };
        assert!(matches!(actions[0], Request::Close { id: 1 }));

        assert!(build_request(
            "batch_actions",
            &json!({ "actions": [{ "tool": "focus", "args": { "id": 1 } }] }),
        )
        .unwrap_err()
        .contains("unsupported"));

        assert!(build_request(
            "batch_actions",
            &json!({ "actions": [{ "tool": "batch_actions", "args": { "actions": [{ "tool": "snapshot" }] } }] }),
        )
        .unwrap_err()
        .contains("nested"));
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
            "render": { "html": "<h1>hi</h1>" },
            "prefetch": { "url": "example.com" },
            "snapshot": {},
            "expect": { "selector": "h1" },
            "click": { "ref": 0 },
            "type": { "ref": 0, "text": "x" },
            "paste": { "ref": 0 },
            "eval": { "js": "1+1" },
            "console": {},
            "net": {},
            "shot": {},
            "scroll": {},
            "wait_load": {},
            "batch": { "actions": [{ "tool": "snapshot" }] },
            "upload": { "selector": "input", "path": "/tmp/x" },
            "challenge": {},
            "motion": {},
            "seek": { "progress": 0.5 },
            "clock": { "action": "pause" },
            "diff": { "id": 1, "other": 2 },
            "show": { "id": 1 },
            "hide": { "id": 1 },
            "press": { "key": "Tab" },
            "close": { "id": 1 },
            "subscribe_events": {},
            "content": {},
            "fill": { "fields": [{ "selector": "input", "value": "x" }] },
            "list_downloads": {},
            "drop_file": { "selector": ".dropzone", "path": "/tmp/x" },
            "auth_context": {},
            "fill_login": {},
            "fork": {},
            "list_forks": {},
            "list_frames": {},
            "try_until": { "alternatives": [{ "tool": "click", "args": { "ref": 0 } }] },
            "scout": { "url": "example.com" },
        });
        for def in tool_definitions() {
            let name = def["name"].as_str().unwrap();
            if name == "verify_ui" {
                // Client-local orchestration, not a daemon Request.
                continue;
            }
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
