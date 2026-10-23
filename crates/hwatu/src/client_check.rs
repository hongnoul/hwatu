// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Justin Hong
//! Client-side `check` and `render` for engines whose daemon has no
//! native one-roundtrip pass.
//!
//! The pass is composed from commands every daemon implements: `open`
//! (headless), `navigate`, `eval`, `shot`, `close`, plus the client
//! `diff` from [`crate::client_diff`]. The reply has the same top-level
//! fields as the GTK daemon's: `url`, `load_ms`, `title`, `total_ms`, and
//! `eval`, `shot`, `diff`, `id` when requested. It costs several socket
//! roundtrips instead of one; the result is the same.

use hwatu_ipc::{LoadStage, OpenMode, Request, Response};
use std::time::Instant;

/// True when a daemon answered that it does not implement `check`.
pub(crate) fn unsupported(response: &Response) -> bool {
    matches!(response, Response::Err { message, .. }
        if message.starts_with("command check is not supported"))
}

/// `render` loads inline markup. Without a native `load_html`, the
/// markup goes in as a `data:` URL, which has an opaque origin, so a
/// `base` cannot be honored.
fn data_url(html: &str) -> String {
    format!(
        "data:text/html;charset=utf-8;base64,{}",
        hwatu_ipc::base64::encode(html.as_bytes())
    )
}

/// Run a `Request::Check` from portable commands. `transact` sends one
/// request to the same daemon.
pub(crate) fn run(
    request: &Request,
    mut transact: impl FnMut(&Request) -> std::io::Result<Response>,
) -> std::io::Result<Response> {
    let Request::Check {
        url,
        render,
        base,
        eval,
        shot,
        shot_path,
        shot_data,
        full,
        baseline,
        baseline_data,
        tolerance,
        min_region_px,
        heatmap,
        heatmap_data,
        until,
        keep,
        timeout_ms,
        viewports,
        baseline_dir,
    } = request
    else {
        unreachable!("client check called with a non-check request");
    };
    let fail = |message: String| Ok(Response::err(message));
    if !viewports.is_empty() || baseline_dir.is_some() {
        return fail("check --viewports is not supported by this engine's daemon yet".into());
    }
    if base.is_some() {
        return fail("render --base is not supported by this engine's daemon yet".into());
    }
    let target = match (url, render) {
        (Some(u), None) => u.clone(),
        (None, Some(html)) => data_url(html),
        _ => return fail("pass exactly one of a url or inline markup".into()),
    };

    let started = Instant::now();
    let opened = transact(&Request::Open {
        url: None,
        app_id: None,
        mode: OpenMode::Headless,
        profile: None,
    })?;
    let id = match &opened {
        Response::Ok {
            window: Some(w), ..
        } => w.id,
        Response::Ok { value: Some(v), .. } if v["window"]["id"].is_u64() => {
            v["window"]["id"].as_u64().unwrap_or(0)
        }
        Response::Err { message, .. } => return fail(message.clone()),
        _ => return fail("check: open returned no window".into()),
    };

    let result = pass(
        &mut transact,
        id,
        &target,
        PassOptions {
            eval: eval.as_deref(),
            shot: *shot || shot_path.is_some() || *shot_data,
            shot_path: shot_path.as_deref(),
            shot_data: *shot_data,
            full: *full,
            baseline: baseline.as_deref(),
            baseline_data: baseline_data.as_deref(),
            tolerance: *tolerance,
            min_region_px: *min_region_px,
            heatmap: heatmap.as_deref(),
            heatmap_data: *heatmap_data,
            until: *until,
            timeout_ms: *timeout_ms,
            started,
        },
    );
    if !*keep {
        let _ = transact(&Request::Close { id });
    }
    let mut value = match result? {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    if *keep {
        value["id"] = serde_json::json!(id);
    }
    if render.is_some() {
        value["rendered"] = serde_json::json!(true);
    }
    Ok(Response::value(value))
}

struct PassOptions<'a> {
    eval: Option<&'a str>,
    shot: bool,
    shot_path: Option<&'a str>,
    shot_data: bool,
    full: bool,
    baseline: Option<&'a str>,
    baseline_data: Option<&'a str>,
    tolerance: Option<u8>,
    min_region_px: Option<u32>,
    heatmap: Option<&'a str>,
    heatmap_data: bool,
    until: LoadStage,
    timeout_ms: Option<u64>,
    started: Instant,
}

fn pass(
    transact: &mut impl FnMut(&Request) -> std::io::Result<Response>,
    id: u64,
    target: &str,
    o: PassOptions,
) -> std::io::Result<Result<serde_json::Value, String>> {
    let loaded = transact(&Request::Navigate {
        id: Some(id),
        url: target.to_string(),
        wait: true,
        until: o.until,
        timeout_ms: o.timeout_ms,
    })?;
    let url = match &loaded {
        Response::Ok {
            window: Some(w), ..
        } => w.url.clone(),
        Response::Ok { value: Some(v), .. } => {
            v["window"]["url"].as_str().unwrap_or(target).to_string()
        }
        Response::Err { message, .. } => return Ok(Err(message.clone())),
        _ => target.to_string(),
    };
    let mut value = serde_json::json!({
        "url": url,
        "load_ms": o.started.elapsed().as_millis() as u64,
    });

    if let Some(js) = o.eval {
        value["eval"] = match transact(&Request::Eval {
            id: Some(id),
            js: js.to_string(),
            timeout_ms: o.timeout_ms,
        })? {
            Response::Ok { value: v, .. } => v.unwrap_or(serde_json::Value::Null),
            Response::Err { message, .. } => serde_json::json!({ "error": message }),
        };
    }
    if o.shot {
        value["shot"] = match transact(&Request::Screenshot {
            id: Some(id),
            path: o.shot_path.map(str::to_string),
            full: o.full,
            data: o.shot_data,
        })? {
            Response::Ok { path: Some(p), .. } => serde_json::json!(p),
            Response::Ok { data: Some(d), .. } => {
                value["shot_data"] = serde_json::json!(d);
                serde_json::Value::Null
            }
            Response::Err { message, .. } => serde_json::json!({ "error": message }),
            _ => serde_json::Value::Null,
        };
        if value["shot"].is_null() {
            value.as_object_mut().map(|m| m.remove("shot"));
        }
    }
    if o.baseline.is_some() || o.baseline_data.is_some() {
        let diff = Request::Diff {
            id,
            other: None,
            baseline: o.baseline.map(str::to_string),
            baseline_data: o.baseline_data.map(str::to_string),
            tolerance: o.tolerance,
            min_region_px: o.min_region_px,
            heatmap: o.heatmap.map(str::to_string),
            heatmap_data: o.heatmap_data,
            full: o.full,
            timeout_ms: o.timeout_ms,
        };
        let mut response = transact(&diff)?;
        if crate::client_diff::unsupported(&response) {
            response = crate::client_diff::run(&diff, &mut *transact)?;
        }
        value["diff"] = match response {
            Response::Ok { value: v, .. } => v.unwrap_or(serde_json::Value::Null),
            Response::Err { message, .. } => serde_json::json!({ "error": message }),
        };
    }
    value["title"] = match transact(&Request::Eval {
        id: Some(id),
        js: "return document.title".into(),
        timeout_ms: o.timeout_ms,
    })? {
        Response::Ok { value: Some(v), .. } => v,
        _ => serde_json::json!(""),
    };
    value["total_ms"] = serde_json::json!(o.started.elapsed().as_millis() as u64);
    Ok(Ok(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_unsupported_reply_only() {
        assert!(unsupported(&Response::err(
            "command check is not supported by the macOS daemon yet (hwatud-mac); supported: ..."
        )));
        assert!(!unsupported(&Response::err(
            "check timed out after 15000 ms"
        )));
    }

    #[test]
    fn render_markup_round_trips_through_data_url() {
        let url = data_url("<p>é</p>");
        let b64 = url
            .strip_prefix("data:text/html;charset=utf-8;base64,")
            .unwrap();
        assert_eq!(
            hwatu_ipc::base64::decode(b64).unwrap(),
            "<p>é</p>".as_bytes()
        );
    }
}
