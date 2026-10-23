// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Justin Hong
//! `hwatu motion --observe`: model the motion the page will not admit to.
//!
//! [`crate::verify::motion`] reads *declared* animation (CSS/WAAPI/
//! CSSOM). Script-driven motion (a `requestAnimationFrame` loop
//! integrating timestamp deltas, the classic marquee/physics pattern)
//! is invisible to all of it. This module observes the live page and
//! *fits models* instead of capturing frames:
//!
//! 1. An injected sampler (driven by the virtual clock from
//!    [`crate::clock`], so it works headless where native rAF never
//!    ticks) finds moving elements: a MutationObserver catches
//!    style/class writers, a two-frame rect diff catches everything
//!    else. It then samples `getBoundingClientRect` per virtual frame
//!    over the observation window, and afterwards "wrap-hunts":
//!    fast-forwards virtual time in coarse chunks until looping tracks
//!    jump back, which pins the loop period without waiting real
//!    minutes.
//! 2. The daemon fits each position series: robust linear velocity
//!    (median of per-frame deltas, immune to loop-wrap outliers),
//!    loop period (observed wrap spacing, or wrap distance / velocity),
//!    oscillation period (autocorrelation), and a cubic-bezier easing
//!    fit for one-shot moves. Every fit carries an r² so an agent
//!    knows how much to trust it.
//!
//! The output is a token-cheap JSON spec (no frames, no pixels) merged
//! with the declared inventory, so one command returns the complete
//! motion picture of a page.

use crate::automation::{self, Reply};
use crate::Daemon;
use hwatu_ipc::Response;
use hwatu_motionfit::fit_payload;
use std::rc::Rc;

/// Default observation window (virtual ms).
const DEFAULT_OBSERVE_MS: u64 = 2500;

/// Sampling script. Placeholder `__OBS_MS__` is substituted before
/// injection (the JS is brace-heavy, so `format!` is a hazard).
/// Requires the virtual clock user script (`window.__hwatu_clock`).
pub const SAMPLE_JS: &str = include_str!("../../../page-scripts/motion-observe.js");

/// Run the full observed-motion pipeline: declared inventory, then
/// sampling under virtual time, then model fitting, merged into one
/// reply.
pub fn motion_observe(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
    observe_ms: Option<u64>,
    timeout_ms: Option<u64>,
    reply: Reply,
) {
    let obs_ms = observe_ms.unwrap_or(DEFAULT_OBSERVE_MS).clamp(200, 10_000);
    // Budget: dense window costs about its virtual length in real time
    // (worst case), the wrap hunt is capped at 20 s real, plus margin.
    let eval_timeout = timeout_ms.unwrap_or(obs_ms + 35_000);
    let daemon2 = daemon.clone();
    // Declared inventory first (cheap, does not advance time)...
    crate::verify::motion_value(
        daemon,
        id,
        Some(5_000),
        Box::new(move |declared_resp| {
            let mut declared = match declared_resp {
                Response::Ok {
                    value: Some(v @ serde_json::Value::Object(_)),
                    ..
                } => v,
                Response::Ok { .. } => serde_json::json!({}),
                err @ Response::Err { .. } => return reply(err),
            };
            // ...then observe under virtual time and fit.
            let js = SAMPLE_JS.replace("__OBS_MS__", &obs_ms.to_string());
            automation::eval(
                &daemon2,
                id,
                js,
                Some(eval_timeout),
                Box::new(move |sample_resp| {
                    let payload = match sample_resp {
                        Response::Ok { value: Some(v), .. } => v,
                        Response::Ok { .. } => {
                            return reply(Response::err("observe returned no data"))
                        }
                        err @ Response::Err { .. } => return reply(err),
                    };
                    if let Some(e) = payload.get("error").and_then(|e| e.as_str()) {
                        return reply(Response::err(e));
                    }
                    let observed = fit_payload(&payload);
                    let meta = serde_json::json!({
                        "window_ms": payload.get("window_ms"),
                        "frames": payload.get("frames"),
                        "wrap_hunt_ms": payload.get("hunted_ms"),
                        "virtual_time": true,
                    });
                    if let Some(map) = declared.as_object_mut() {
                        map.insert("observed".into(), serde_json::Value::Array(observed));
                        map.insert("observed_meta".into(), meta);
                    }
                    reply(Response::value(declared));
                }),
            );
        }),
    );
}
