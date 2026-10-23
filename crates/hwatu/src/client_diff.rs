// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Justin Hong
//! Client-side `diff` for engines whose daemon has no native pixel diff.
//!
//! The daemon captures with `shot` (inline PNG bytes); the client diffs
//! with the same `hwatu-pixeldiff` code the GTK daemon uses, so the
//! report shape and scoring are identical across engines.

use hwatu_ipc::{Request, Response};
use hwatu_pixeldiff::Frame;

/// True when a daemon answered that it does not implement `diff`.
pub(crate) fn unsupported(response: &Response) -> bool {
    matches!(response, Response::Err { message, .. }
        if message.starts_with("command diff is not supported"))
}

/// Run a `Request::Diff` by capturing and diffing on the client.
/// `transact` sends one request to the same daemon.
pub(crate) fn run(
    request: &Request,
    mut transact: impl FnMut(&Request) -> std::io::Result<Response>,
) -> std::io::Result<Response> {
    let Request::Diff {
        id,
        other,
        baseline,
        baseline_data,
        tolerance,
        min_region_px,
        heatmap,
        heatmap_data,
        full,
        ..
    } = request
    else {
        unreachable!("client diff called with a non-diff request");
    };
    let fail = |message: String| Ok(Response::err(message));
    let inputs = usize::from(other.is_some())
        + usize::from(baseline.is_some())
        + usize::from(baseline_data.is_some());
    if inputs != 1 {
        return fail("pass exactly one of --other, --baseline, or inline baseline data".into());
    }

    let mut capture = |window: u64| -> std::io::Result<Result<Frame, String>> {
        let response = transact(&Request::Screenshot {
            id: Some(window),
            path: None,
            full: *full,
            data: true,
        })?;
        Ok(match response {
            Response::Ok {
                data: Some(encoded),
                ..
            } => hwatu_ipc::base64::decode(&encoded)
                .map_err(|e| format!("screenshot of window {window} is not valid base64: {e}"))
                .and_then(|bytes| Frame::from_png_bytes(&bytes)),
            // A daemon without inline data wrote a temp file on this host.
            Response::Ok { path: Some(p), .. } => {
                let frame = std::fs::read(&p)
                    .map_err(|e| format!("cannot read screenshot {p}: {e}"))
                    .and_then(|bytes| Frame::from_png_bytes(&bytes));
                let _ = std::fs::remove_file(&p);
                frame
            }
            Response::Err { message, .. } => Err(message),
            _ => Err(format!("screenshot of window {window} returned no image")),
        })
    };

    let a = match capture(*id)? {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let b = if let Some(other) = other {
        match capture(*other)? {
            Ok(frame) => frame,
            Err(e) => return fail(e),
        }
    } else if let Some(path) = baseline {
        match std::fs::read(path)
            .map_err(|e| format!("cannot read baseline {path}: {e}"))
            .and_then(|bytes| Frame::from_png_bytes(&bytes))
        {
            Ok(frame) => frame,
            Err(e) => return fail(e),
        }
    } else {
        let encoded = baseline_data.as_deref().unwrap_or_default();
        match hwatu_ipc::base64::decode(encoded)
            .map_err(|e| format!("inline baseline is not valid base64: {e}"))
            .and_then(|bytes| Frame::from_png_bytes(&bytes))
        {
            Ok(frame) => frame,
            Err(e) => return fail(e),
        }
    };

    let engine = match transact(&Request::Ping)? {
        Response::Ok { value: Some(v), .. } => {
            let name = v["engine"]["name"].as_str().unwrap_or("unknown");
            let version = v["engine"]["version"].as_str().unwrap_or("");
            format!("{name} {version}").trim().to_string()
        }
        _ => "unknown".into(),
    };
    let (mut value, heatmap_png) = match hwatu_pixeldiff::report(
        &a,
        &b,
        tolerance.unwrap_or(8),
        *min_region_px,
        heatmap.is_some() || *heatmap_data,
        *full,
        &engine,
    ) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    if let (Some(path), Some(png)) = (heatmap, heatmap_png.as_ref()) {
        if let Err(e) = std::fs::write(path, png) {
            return fail(format!("heatmap write to {path} failed: {e}"));
        }
        value["heatmap"] = serde_json::Value::String(path.clone());
    }
    if *heatmap_data {
        if let Some(png) = heatmap_png {
            value["heatmap_data"] = serde_json::Value::String(hwatu_ipc::base64::encode(&png));
        }
    }
    Ok(Response::value(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            let mut wr = enc.write_header().unwrap();
            let data: Vec<u8> = (0..w * h).flat_map(|_| rgb).collect();
            wr.write_image_data(&data).unwrap();
        }
        out
    }

    fn diff_req(other: Option<u64>, baseline_data: Option<String>) -> Request {
        Request::Diff {
            id: 1,
            other,
            baseline: None,
            baseline_data,
            tolerance: None,
            min_region_px: None,
            heatmap: None,
            heatmap_data: false,
            full: false,
            timeout_ms: None,
        }
    }

    #[test]
    fn detects_unsupported_reply_only() {
        assert!(unsupported(&Response::err(
            "command diff is not supported by the macOS daemon yet (hwatud-mac); supported: ..."
        )));
        assert!(!unsupported(&Response::err("no window 3")));
    }

    #[test]
    fn diffs_two_windows_from_inline_shots() {
        let white = hwatu_ipc::base64::encode(&png(64, 64, [255, 255, 255]));
        let red = hwatu_ipc::base64::encode(&png(64, 64, [255, 0, 0]));
        let mut seen = vec![];
        let resp = run(&diff_req(Some(2), None), |r| {
            seen.push(format!("{r:?}"));
            Ok(match r {
                Request::Screenshot {
                    id: Some(1),
                    data: true,
                    ..
                } => Response::data(white.clone()),
                Request::Screenshot {
                    id: Some(2),
                    data: true,
                    ..
                } => Response::data(red.clone()),
                Request::Ping => Response::value(serde_json::json!({
                    "engine": {"name": "wkwebview", "version": "1"}
                })),
                other => panic!("unexpected request {other:?}"),
            })
        })
        .unwrap();
        let Response::Ok { value: Some(v), .. } = resp else {
            panic!("{resp:?}")
        };
        assert_eq!(v["match_percent"], 0.0);
        assert_eq!(v["significant_regions"], 1);
        assert_eq!(v["envelope"]["engine"], "wkwebview 1");
    }

    #[test]
    fn baseline_and_rejects_ambiguous_inputs() {
        let white = hwatu_ipc::base64::encode(&png(8, 8, [255, 255, 255]));
        let shot = white.clone();
        let resp = run(&diff_req(None, Some(white)), |r| {
            Ok(match r {
                Request::Screenshot { .. } => Response::data(shot.clone()),
                _ => Response::value(serde_json::json!({})),
            })
        })
        .unwrap();
        let Response::Ok { value: Some(v), .. } = resp else {
            panic!("{resp:?}")
        };
        assert_eq!(v["match_percent"], 100.0);

        let resp = run(&diff_req(None, None), |_| panic!("no request expected")).unwrap();
        assert!(matches!(resp, Response::Err { .. }));
    }
}
