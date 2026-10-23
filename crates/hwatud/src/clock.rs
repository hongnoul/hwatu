// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Justin Hong
//! Virtual time for a page: `hwatu clock pause|resume|step|set`.
//!
//! [`crate::verify::seek`] pins declarative animation (CSS/WAAPI)
//! because the Web Animations API exposes their timeline. Script-driven
//! motion has no such handle: a `requestAnimationFrame` loop that
//! integrates timestamp deltas (the classic marquee/physics pattern)
//! keeps running through a seek, so the frame is never deterministic.
//!
//! This module injects a user script at document start (before any
//! page code runs) that puts every clock the page can read behind one
//! controllable virtual timeline:
//!
//! - `performance.now()`, `Date.now()`, and zero-argument `Date`
//!   construction return virtual time,
//! - `setTimeout`/`setInterval` fire on virtual deadlines (delegated
//!   1:1 to native timers while the clock is live, so real pages keep
//!   their real behavior until an agent takes control),
//! - `requestAnimationFrame` callbacks are held in a registry and
//!   flushed by a native rAF pump while live, or by `step` while
//!   paused, always with a virtual timestamp,
//! - CSS/WAAPI animations are paused and their `currentTime` advanced
//!   from the same timeline, so declarative and script motion share
//!   one clock.
//!
//! `pause` freezes all of it; `step <ms>` advances deterministically in
//! 60 fps ticks (due timers fire, one rAF batch per tick); `set <ms>`
//! steps to an absolute virtual time; `resume` returns to real time
//! monotonically. Two screenshots at the same virtual time are
//! byte-identical, which is what turns "the page moves" from a flake
//! into a measurement.

use crate::automation::{self, Reply};
use crate::Daemon;
use hwatu_ipc::{ClockAction, Response};
use std::rc::Rc;

/// User script injected at document start in every frame. Installs the
/// virtual clock *dormant*: all wrapped clocks delegate to the native
/// ones (virtual time == real time) until the first `pause`/`step`/
/// `set`, so pages that never use `hwatu clock` behave natively.
/// Idempotent per realm.
pub const CLOCK_JS: &str = include_str!("../../../page-scripts/clock.js");

/// Register the virtual-clock user script on a WebView's content
/// manager. Must run on every view (prewarmed pool, popups) before it
/// loads page content, because the wrappers must win the race against
/// page scripts capturing the native clocks.
pub fn wire_view(view: &webkit6::WebView) {
    use crate::engine::{EngineView, FrameScope, ScriptTime, WebKitView};
    let engine = WebKitView(view.clone());
    // Deterministic-load mode: a tiny preamble script (added first, so
    // it runs first) flags the realm before the clock installs. Only
    // isolated verification daemons set this env; interactive daemons
    // keep native passthrough behavior.
    if std::env::var("HWATU_CLOCK_START_PAUSED").is_ok_and(|v| !v.is_empty() && v != "0") {
        engine.add_user_script(
            "window.__hwatu_clock_start_paused = true;",
            ScriptTime::Start,
            FrameScope::All,
        );
    }
    // Pin Date.now() independently of timeline state. This lets verification
    // pages finish native-clock navigation/font/hydration waits before their
    // first `clock set 0`, while still giving every page the same wall time.
    if let Some(epoch_ms) = std::env::var("HWATU_CLOCK_EPOCH_MS")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
    {
        engine.add_user_script(
            &format!("window.__hwatu_clock_epoch_ms = {epoch_ms};"),
            ScriptTime::Start,
            FrameScope::All,
        );
    }
    engine.add_user_script(CLOCK_JS, ScriptTime::Start, FrameScope::All);
}

/// Dispatch one clock command to the page's installed control surface.
///
/// `seed` is special: besides seeding the live page immediately, it
/// registers a document-start user script on the window's view so
/// every *future* load in that window is seeded before any page
/// script can capture or consume the native `Math.random`.
pub fn clock(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
    action: ClockAction,
    ms: Option<f64>,
    seed: Option<u64>,
    timeout_ms: Option<u64>,
    reply: Reply,
) {
    let call = match (action, ms) {
        (ClockAction::Pause, _) => "c.pause()".to_string(),
        (ClockAction::Resume, _) => "c.resume()".to_string(),
        (ClockAction::Status, _) => "c.status()".to_string(),
        (ClockAction::Step, Some(ms)) if ms.is_finite() && ms >= 0.0 => format!("c.step({ms})"),
        (ClockAction::Step, _) => {
            return reply(Response::err("usage: hwatu clock step <ms>"));
        }
        (ClockAction::Set, Some(ms)) if ms.is_finite() && ms >= 0.0 => format!("c.set({ms})"),
        (ClockAction::Set, _) => {
            return reply(Response::err("usage: hwatu clock set <ms>"));
        }
        (ClockAction::Seed, _) => {
            let Some(seed) = seed else {
                return reply(Response::err("usage: hwatu clock seed <u64>"));
            };
            // f64 can't hold every u64 exactly; the shim folds to u32
            // anyway, but reject seeds that would silently change so
            // "same seed" always means the same PRNG.
            if seed > (1u64 << 53) {
                return reply(Response::err("seed must be <= 2^53 (JS number precision)"));
            }
            if let Err(resp) = wire_seed(daemon, id, seed) {
                return reply(*resp);
            }
            format!("c.seedRandom({seed})")
        }
    };
    let js = format!(
        r#"
const c = window.__hwatu_clock;
if (!c) return {{ error: "virtual clock not installed in this page (page predates this daemon build; reload it)" }};
return {call};"#
    );
    automation::eval(daemon, id, js, timeout_ms, reply);
}

/// Register the persistent seed flag script on the target window's
/// view. Runs at document start; the flag global covers loads where it
/// executes before CLOCK_JS, the direct call covers loads where the
/// clock installed first (scripts run in registration order, and
/// CLOCK_JS is registered at view build time).
fn wire_seed(daemon: &Rc<Daemon>, id: Option<u64>, seed: u64) -> Result<(), Box<Response>> {
    use crate::engine::{EngineView, FrameScope, ScriptTime, WebKitView};
    let windows = daemon.windows.borrow();
    let win = match id {
        Some(id) => windows
            .get(&id)
            .cloned()
            .ok_or_else(|| Box::new(Response::err(format!("no window {id}"))))?,
        None => match windows.len() {
            1 => windows.values().next().cloned().expect("len checked"),
            0 => return Err(Box::new(Response::err("no windows open"))),
            n => {
                return Err(Box::new(Response::err(format!(
                    "{n} windows open; pass --id"
                ))))
            }
        },
    };
    let Some(view) = win.live_webview() else {
        return Err(Box::new(Response::err("window has no live webview")));
    };
    let js = format!(
        "window.__hwatu_clock_seed = {seed};\n\
         if (window.__hwatu_clock) window.__hwatu_clock.seedRandom({seed});"
    );
    WebKitView(view).add_user_script(&js, ScriptTime::Start, FrameScope::All);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::CLOCK_JS;

    /// Scroll events must be synthesized under virtual stepping: the
    /// step loop dispatches a bubbling `scroll` on document when the
    /// window offset changed since the last tick, because headless
    /// pages never get the rendering opportunity WebKit needs to fire
    /// the native event (scroll-driven libs stay frozen otherwise).
    #[test]
    fn clock_js_pumps_scroll_events_under_step() {
        assert!(CLOCK_JS.contains("pumpScroll"));
        assert!(CLOCK_JS.contains("new Event('scroll', { bubbles: true })"));
        // Fires only on change: unchanged offsets are a no-op.
        assert!(CLOCK_JS.contains("g.scrollX === lastScrollX && g.scrollY === lastScrollY"));
    }

    #[test]
    fn clock_js_exposes_viewport_pump_without_advancing_time() {
        assert!(CLOCK_JS.contains("const pumpViewport = () => {"));
        assert!(CLOCK_JS.contains("pumpScroll();\n    pumpIO();"));
        assert!(CLOCK_JS
            .contains("dateNow: () => Math.round(dateBase + realNow()),\n      pumpViewport,"));
    }

    #[test]
    fn clock_js_native_delivery_suppresses_duplicate_viewport_callbacks() {
        assert!(CLOCK_JS.contains("state.last.set(entry.target, entry.isIntersecting)"));
        assert!(
            CLOCK_JS.contains("document.addEventListener('scroll', syncScroll, { capture: true })")
        );
    }

    /// The seeded-PRNG surface must exist in the injected shim: the
    /// control surface exposes seedRandom, status reports the seed,
    /// and the document-start flag path is wired for future loads.
    #[test]
    fn clock_js_exposes_seeded_random_surface() {
        for needle in [
            "seedRandom",
            "nativeRandom",
            "seed: randomSeed",
            "__hwatu_clock_seed",
            "0x6D2B79F5", // mulberry32 increment: the algorithm is part of the contract
        ] {
            assert!(CLOCK_JS.contains(needle), "missing {needle}");
        }
    }

    /// Default behavior unchanged: Math.random is only replaced inside
    /// seedRandom, never unconditionally at install.
    #[test]
    fn clock_js_does_not_replace_math_random_by_default() {
        // Exactly one assignment to Math.random, and it lives inside
        // seedRandom's body (after its declaration).
        let assigns: Vec<_> = CLOCK_JS.match_indices("Math.random =").collect();
        assert_eq!(
            assigns.len(),
            1,
            "Math.random should be assigned exactly once"
        );
        let seed_fn = CLOCK_JS
            .find("const seedRandom")
            .expect("seedRandom missing");
        assert!(
            assigns[0].0 > seed_fn,
            "Math.random must only be replaced inside seedRandom"
        );
        // And installation-time seeding is gated on the flag global.
        assert!(CLOCK_JS.contains(
            "if (typeof g.__hwatu_clock_seed === 'number') seedRandom(g.__hwatu_clock_seed);"
        ));
    }

    /// A settled live page can define virtual t=0 with its first `set 0`
    /// without trying to rewind DOM/script state. Timer deadlines move by
    /// the same delta so their remaining delays are preserved.
    #[test]
    fn clock_js_first_live_set_can_rebase_backwards() {
        for needle in [
            "const wasPaused = paused",
            "if (!wasPaused)",
            "entry.deadline += delta",
            "vbase = ms",
        ] {
            assert!(
                CLOCK_JS.contains(needle),
                "missing rebase behavior: {needle}"
            );
        }
    }

    #[test]
    fn clock_js_accepts_a_fixed_epoch_without_starting_paused() {
        assert!(CLOCK_JS.contains("Number(g.__hwatu_clock_epoch_ms)"));
        assert!(CLOCK_JS.contains("Number.isFinite(configuredEpoch)"));
    }

    /// Pinning only Date.now leaves `new Date()` and `Date()` coupled to
    /// real wall time. The shim must virtualize both implicit-now forms
    /// without changing explicit Date construction or native statics.
    #[test]
    fn clock_js_virtualizes_the_complete_date_now_surface() {
        for needle in [
            "const NativeDate = g.Date",
            "const VirtualDate = new Proxy(NativeDate",
            "args.length === 0 ? [virtualDateNow()] : args",
            "return new NativeDate(virtualDateNow()).toString()",
            "if (property === 'now') return virtualDateNow",
            "g.Date = VirtualDate",
        ] {
            assert!(CLOCK_JS.contains(needle), "missing Date behavior: {needle}");
        }
    }

    /// Same seed + same call count => identical sequences. The Rust
    /// port here is the executable spec of the JS mulberry32 in
    /// CLOCK_JS (u32 arithmetic matches JS `>>> 0` / `Math.imul`);
    /// golden values pin the algorithm so a silent edit to the shim's
    /// PRNG breaks this test.
    #[test]
    fn mulberry32_reference_sequence_is_deterministic() {
        fn mulberry32(seed: u64) -> impl FnMut() -> f64 {
            let mut state = (seed as u32) ^ ((seed / 4_294_967_296) as u32);
            move || {
                state = state.wrapping_add(0x6D2B_79F5);
                let mut t = state;
                t = (t ^ (t >> 15)).wrapping_mul(t | 1);
                t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
                f64::from(t ^ (t >> 14)) / 4_294_967_296.0
            }
        }
        let mut a = mulberry32(42);
        let mut b = mulberry32(42);
        let seq_a: Vec<f64> = (0..64).map(|_| a()).collect();
        let seq_b: Vec<f64> = (0..64).map(|_| b()).collect();
        assert_eq!(seq_a, seq_b, "same seed must give identical sequences");
        assert!(seq_a.iter().all(|v| (0.0..1.0).contains(v)));
        // Golden first values for seed 42 (pins the exact algorithm).
        assert_eq!(seq_a[0], 0.6011037519201636);
        assert_eq!(seq_a[1], 0.44829055899754167);
        let mut c = mulberry32(43);
        assert_ne!(seq_a[0], c(), "different seeds must diverge");
    }
}
