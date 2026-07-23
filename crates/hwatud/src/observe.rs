//! `hwatu motion --observe`: model the motion the page will not admit to.
//!
//! [`crate::verify::motion`] reads *declared* animation (CSS/WAAPI/
//! CSSOM). Script-driven motion — a `requestAnimationFrame` loop
//! integrating timestamp deltas, the classic marquee/physics pattern —
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
use std::rc::Rc;

/// Default observation window (virtual ms).
const DEFAULT_OBSERVE_MS: u64 = 2500;

/// Sampling script. Placeholder `__OBS_MS__` is substituted before
/// injection (the JS is brace-heavy, so `format!` is a hazard).
/// Requires the virtual clock user script (`window.__hwatu_clock`).
const SAMPLE_JS: &str = r#"
const c = window.__hwatu_clock;
if (!c) return { error: "virtual clock not installed in this page (reload it under this daemon build)" };
const OBS_MS = __OBS_MS__;
const TICK = 1000 / 60;
const MAX_TRACKS = 24;
const MAX_ELEMENTS = 4000;
const sel = (el) => {
  if (!el || !el.tagName) return null;
  let s = el.tagName.toLowerCase();
  if (el.id) return s + '#' + el.id;
  if (el.classList && el.classList.length) s += '.' + [...el.classList].slice(0, 3).join('.');
  return s;
};

// -- discovery: who moves? -------------------------------------------
// MutationObserver catches style/class writers (covers styles the
// rect diff below would miss, e.g. an element about to start moving);
// the rect diff over a 3-frame virtual step catches everything that
// actually moved, however it is driven (transform, layout, canvas
// containers repositioned by script).
const mutated = new Set();
const mo = new MutationObserver(recs => {
  for (const r of recs) {
    if (r.target && r.target.nodeType === 1) mutated.add(r.target);
  }
});
mo.observe(document.documentElement, { attributes: true, subtree: true, attributeFilter: ['style', 'class'] });

const all = [];
for (const el of document.querySelectorAll('body *')) {
  if (all.length >= MAX_ELEMENTS) break;
  const r = el.getBoundingClientRect();
  if (r.width > 2 && r.height > 2) { all.push([el, r.x, r.y]); }
}
c.pause();
// Real macrotask yield: React (and friends) schedule state updates on
// scheduler macrotasks, so an IO-gated animation ("start when in
// view") only arms after one. `await null` would only drain
// microtasks. MessageChannel, not setTimeout: hidden pages throttle
// DOM timers to ~1s, which would turn 30 yields into 30 real seconds.
const yieldReal = () => new Promise(r => {
  const ch = new MessageChannel();
  ch.port1.onmessage = () => r();
  ch.port2.postMessage(0);
});
c.step(TICK * 2);
await yieldReal(); // IO pump delivered; let gated starters arm
c.step(TICK * 3);
await yieldReal();

const movingSet = new Set();
for (const [el, x0, y0] of all) {
  const r = el.getBoundingClientRect();
  if (Math.abs(r.x - x0) > 0.5 || Math.abs(r.y - y0) > 0.5) movingSet.add(el);
}
// Movement is inherited: when a container moves, every descendant
// "moves". Track only topmost movers so the marquee is one track,
// not 37.
const topmost = [];
for (const el of movingSet) {
  let p = el.parentElement, top = true;
  while (p) { if (movingSet.has(p)) { top = false; break; } p = p.parentElement; }
  if (top) topmost.push(el);
}
// Style-mutated elements that have not moved yet may be one-shot
// movers about to fire; observe them too, movers first.
for (const el of mutated) {
  if (topmost.length >= MAX_TRACKS) break;
  if (!el.getBoundingClientRect || movingSet.has(el)) continue;
  let p = el.parentElement, covered = false;
  while (p) { if (movingSet.has(p)) { covered = true; break; } p = p.parentElement; }
  if (!covered) topmost.push(el);
}
mo.disconnect();

const tracks = topmost.slice(0, MAX_TRACKS).map(el => {
  const st = getComputedStyle(el);
  return {
    el,
    target: sel(el),
    property: st.transform && st.transform !== 'none' ? 'transform' : 'layout',
    scroll_w: el.scrollWidth, scroll_h: el.scrollHeight,
    xs: [], ys: [],
  };
});

// -- dense window: one sample per virtual frame ------------------------
const frames = Math.max(2, Math.min(Math.round(OBS_MS / TICK), 600));
for (let i = 0; i < frames; i++) {
  for (const t of tracks) {
    const r = t.el.getBoundingClientRect();
    t.xs.push(Math.round(r.x * 100) / 100);
    t.ys.push(Math.round(r.y * 100) / 100);
  }
  c.step(TICK);
  if (i % 5 === 4) await yieldReal(); else await null;
}

// -- wrap hunt: fast-forward to catch loop periods ---------------------
// A marquee's loop period is often minutes; nobody observes that in
// real time. Under virtual time we can step coarse chunks and watch
// for the position to jump against its own linear prediction.
const CHUNK = 100; // virtual ms between hunt samples
const HUNT_REAL_BUDGET_MS = 20000;
const huntStart = c.native.now();
let hunted = [];
for (const t of tracks) {
  for (const axis of ['x', 'y']) {
    const s = axis === 'x' ? t.xs : t.ys;
    const n = s.length;
    if (n < 8) continue;
    const deltas = [];
    for (let i = 1; i < n; i++) deltas.push(s[i] - s[i - 1]);
    const nz = deltas.filter(d => Math.abs(d) > 0.01);
    if (!nz.length) continue;
    const pos = nz.filter(d => d > 0).length;
    const mono = Math.max(pos, nz.length - pos) / nz.length;
    const v = (s[n - 1] - s[0]) / ((n - 1) * TICK); // px per virtual ms
    // Monotonic, still moving at the end: a loop candidate.
    const tail = Math.abs(s[n - 1] - s[n - 2]);
    if (mono >= 0.9 && Math.abs(v) * 1000 > 1 && tail > 0.01) {
      const extent = Math.max(t.scroll_w, t.scroll_h);
      const capMs = Math.min(200000, Math.max(20000, 2.5 * extent / Math.abs(v)));
      hunted.push({ t, axis, v, last: s[n - 1], capMs, wrap1: null, wrap2: null, done: false });
      break; // one hunt axis per track
    }
  }
}
let vtime = 0;
while (hunted.some(h => !h.done) && c.native.now() - huntStart < HUNT_REAL_BUDGET_MS) {
  if (!hunted.some(h => !h.done && vtime < h.capMs)) break;
  c.step(CHUNK);
  vtime += CHUNK;
  await yieldReal();
  for (const h of hunted) {
    if (h.done) continue;
    if (vtime >= h.capMs) { h.done = true; continue; }
    const r = h.t.el.getBoundingClientRect();
    const p = h.axis === 'x' ? r.x : r.y;
    const pred = h.last + h.v * CHUNK;
    if (Math.abs(p - pred) > Math.max(24, Math.abs(h.v * CHUNK) * 4)) {
      if (h.wrap1 === null) h.wrap1 = { t_ms: vtime, jump: p - pred };
      else { h.wrap2 = { t_ms: vtime }; h.done = true; }
    }
    h.last = p;
  }
}
for (const h of hunted) {
  h.t.wrap = { axis: h.axis, wrap1: h.wrap1, wrap2: h.wrap2 };
}
c.resume();
return {
  dt_ms: TICK,
  frames,
  window_ms: Math.round(frames * TICK),
  hunted_ms: vtime,
  tracks: tracks.map(t => ({
    target: t.target, property: t.property,
    scroll_w: t.scroll_w, scroll_h: t.scroll_h,
    xs: t.xs, ys: t.ys, wrap: t.wrap || null,
  })),
};"#;

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

// ---- model fitting ---------------------------------------------------

/// One axis of one sampled element, ready to fit.
struct Series<'a> {
    positions: &'a [f64],
    /// Virtual ms between samples.
    dt_ms: f64,
}

/// Fitted model for one track, straight into output JSON.
fn fit_payload(payload: &serde_json::Value) -> Vec<serde_json::Value> {
    let dt_ms = payload.get("dt_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let window_ms = payload
        .get("window_ms")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let Some(tracks) = payload.get("tracks").and_then(|v| v.as_array()) else {
        return vec![];
    };
    if dt_ms <= 0.0 {
        return vec![];
    }
    let mut out = vec![];
    for track in tracks {
        let xs = num_array(track.get("xs"));
        let ys = num_array(track.get("ys"));
        let (axis, positions) = match (range(&xs), range(&ys)) {
            (rx, ry) if rx < 1.0 && ry < 1.0 => continue, // static
            (rx, ry) if rx >= ry => ("x", &xs),
            _ => ("y", &ys),
        };
        let series = Series { positions, dt_ms };
        let wrap = track.get("wrap").filter(|w| {
            w.get("axis").and_then(|a| a.as_str()) == Some(axis) && !w["wrap1"].is_null()
        });
        let Some(mut fit) = fit_series(&series, wrap, window_ms) else {
            continue;
        };
        if let Some(map) = fit.as_object_mut() {
            map.insert(
                "target".into(),
                track.get("target").cloned().unwrap_or_default(),
            );
            map.insert(
                "property".into(),
                track.get("property").cloned().unwrap_or_default(),
            );
            map.insert("axis".into(), serde_json::json!(axis));
            map.insert("source".into(), serde_json::json!("observed"));
        }
        out.push(fit);
    }
    out
}

fn num_array(v: Option<&serde_json::Value>) -> Vec<f64> {
    v.and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_f64()).collect())
        .unwrap_or_default()
}

fn range(s: &[f64]) -> f64 {
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for &v in s {
        lo = lo.min(v);
        hi = hi.max(v);
    }
    if s.is_empty() {
        0.0
    } else {
        hi - lo
    }
}

/// Fit one position series to linear / periodic / bezier. `wrap` is
/// the wrap-hunt result for this axis, if the sampler saw the track
/// loop after the dense window.
fn fit_series(
    series: &Series,
    wrap: Option<&serde_json::Value>,
    window_ms: f64,
) -> Option<serde_json::Value> {
    let s = series.positions;
    let n = s.len();
    if n < 8 {
        return None;
    }
    let dt = series.dt_ms;
    let deltas: Vec<f64> = s.windows(2).map(|w| w[1] - w[0]).collect();
    let med = median(&deltas);
    let mad = median(&deltas.iter().map(|d| (d - med).abs()).collect::<Vec<_>>());
    // Wraps inside the dense window: deltas wildly off the median.
    let jump_threshold = (10.0 * mad).max(6.0).max(med.abs() * 5.0);
    let jumps: Vec<usize> = deltas
        .iter()
        .enumerate()
        .filter(|(_, d)| (*d - med).abs() > jump_threshold)
        .map(|(i, _)| i)
        .collect();

    // One-shot: at rest at both ends, motion in the middle.
    let edge = (n / 10).max(2);
    let rest = |slice: &[f64]| slice.iter().all(|d: &f64| d.abs() < 0.05);
    let head_rest = rest(&deltas[..edge]);
    let tail_rest = rest(&deltas[deltas.len() - edge..]);
    let peak = deltas.iter().fold(0.0f64, |m, d| m.max(d.abs()));
    if head_rest && tail_rest && peak > 0.3 {
        return fit_one_shot(s, dt);
    }

    // Unwrap the series (add back the jump at each wrap) so a looping
    // marquee fits as one straight line.
    let mut unwrapped = Vec::with_capacity(n);
    let mut offset = 0.0;
    unwrapped.push(s[0]);
    for (i, d) in deltas.iter().enumerate() {
        if jumps.contains(&i) {
            offset -= d - med; // remove the jump, keep the typical step
        }
        unwrapped.push(s[i + 1] + offset);
    }
    let (slope, intercept, r2) = linear_fit(&unwrapped, dt);
    let velocity = slope * 1000.0; // px per second

    if r2 > 0.9 && velocity.abs() > 0.5 {
        let mut out = serde_json::json!({
            "model": "linear",
            "velocity_px_s": round2(velocity),
            "fit_r2": round4(r2),
        });
        let map = out.as_object_mut().expect("object literal");
        // Loop period: observed wrap spacing inside the window, else
        // wrap-hunt evidence after it, else none (honest absence).
        if jumps.len() >= 2 {
            let spans: Vec<f64> = jumps.windows(2).map(|w| (w[1] - w[0]) as f64 * dt).collect();
            map.insert(
                "period_s".into(),
                serde_json::json!(round2(median(&spans) / 1000.0)),
            );
        } else if let Some(w) = wrap {
            let wrap1_t = w["wrap1"]["t_ms"].as_f64();
            let wrap1_jump = w["wrap1"]["jump"].as_f64();
            let wrap2_t = w["wrap2"]["t_ms"].as_f64();
            let period_ms = match (wrap1_t, wrap2_t, wrap1_jump) {
                (Some(t1), Some(t2), _) => Some(t2 - t1),
                (Some(_), None, Some(jump)) if slope.abs() > 1e-9 => Some((jump / slope).abs()),
                _ => None,
            };
            if let Some(p) = period_ms.filter(|p| *p > 0.0) {
                map.insert("period_s".into(), serde_json::json!(round2(p / 1000.0)));
                if let Some(t1) = wrap1_t {
                    // Seconds already elapsed in the current cycle at
                    // the start of observation.
                    let to_first = window_ms + t1;
                    let phase = (p - (to_first % p)) % p;
                    map.insert("phase_s".into(), serde_json::json!(round2(phase / 1000.0)));
                }
            }
        }
        let _ = intercept;
        return Some(out);
    }

    // Oscillation: no net drift, self-similar. Autocorrelation peak
    // gives the period; its height is the confidence.
    if let Some((lag, peak)) = autocorr_period(&unwrapped) {
        if peak > 0.5 {
            return Some(serde_json::json!({
                "model": "periodic",
                "period_s": round2(lag as f64 * dt / 1000.0),
                "amplitude_px": round2(range(s) / 2.0),
                "fit_r2": round4(peak),
            }));
        }
    }

    // Motion we cannot name: report the linear fit with its honest r².
    Some(serde_json::json!({
        "model": "linear",
        "velocity_px_s": round2(velocity),
        "fit_r2": round4(r2),
    }))
}

/// One-shot move: normalize to a progress curve and fit a CSS cubic
/// bezier to it. Returns duration/distance/easing/r².
fn fit_one_shot(s: &[f64], dt: f64) -> Option<serde_json::Value> {
    let n = s.len();
    // Trim the resting head and tail: the move is the middle span.
    let deltas: Vec<f64> = s.windows(2).map(|w| w[1] - w[0]).collect();
    let start = deltas.iter().position(|d| d.abs() > 0.05)?;
    let end = deltas.iter().rposition(|d| d.abs() > 0.05)? + 1;
    if end <= start + 3 || end > n - 1 {
        return None;
    }
    let p0 = s[start];
    let p1 = s[end];
    let dist = p1 - p0;
    if dist.abs() < 1.0 {
        return None;
    }
    let progress: Vec<f64> = s[start..=end].iter().map(|p| (p - p0) / dist).collect();
    let m = progress.len();
    let (bez, sse) = fit_bezier(&progress);
    let mean = progress.iter().sum::<f64>() / m as f64;
    let sst: f64 = progress.iter().map(|p| (p - mean).powi(2)).sum();
    let r2 = if sst > 0.0 { 1.0 - sse / sst } else { 0.0 };
    Some(serde_json::json!({
        "model": "bezier",
        "duration_ms": round2((end - start) as f64 * dt),
        "distance_px": round2(dist),
        "easing": format!(
            "cubic-bezier({}, {}, {}, {})",
            round2(bez[0]), round2(bez[1]), round2(bez[2]), round2(bez[3])
        ),
        "fit_r2": round4(r2),
    }))
}

/// Least-squares linear fit of `s[i]` against `t = i * dt`.
/// Returns (slope px/ms, intercept, r²).
fn linear_fit(s: &[f64], dt: f64) -> (f64, f64, f64) {
    let n = s.len() as f64;
    let mut sx = 0.0;
    let mut sy = 0.0;
    let mut sxx = 0.0;
    let mut sxy = 0.0;
    for (i, &y) in s.iter().enumerate() {
        let x = i as f64 * dt;
        sx += x;
        sy += y;
        sxx += x * x;
        sxy += x * y;
    }
    let denom = n * sxx - sx * sx;
    if denom.abs() < 1e-12 {
        return (0.0, s.first().copied().unwrap_or(0.0), 0.0);
    }
    let slope = (n * sxy - sx * sy) / denom;
    let intercept = (sy - slope * sx) / n;
    let mean = sy / n;
    let mut sse = 0.0;
    let mut sst = 0.0;
    for (i, &y) in s.iter().enumerate() {
        let x = i as f64 * dt;
        sse += (y - (slope * x + intercept)).powi(2);
        sst += (y - mean).powi(2);
    }
    let r2 = if sst > 1e-12 { 1.0 - sse / sst } else { 1.0 };
    (slope, intercept, r2)
}

fn median(s: &[f64]) -> f64 {
    if s.is_empty() {
        return 0.0;
    }
    let mut v: Vec<f64> = s.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = v.len() / 2;
    if v.len() % 2 == 0 {
        (v[mid - 1] + v[mid]) / 2.0
    } else {
        v[mid]
    }
}

/// Normalized autocorrelation of the mean-removed series; returns the
/// best (lag, peak) after the correlation has first decayed, or None
/// for series that never decorrelate (monotonic drift).
fn autocorr_period(s: &[f64]) -> Option<(usize, f64)> {
    let n = s.len();
    if n < 16 {
        return None;
    }
    let mean = s.iter().sum::<f64>() / n as f64;
    let d: Vec<f64> = s.iter().map(|v| v - mean).collect();
    let var: f64 = d.iter().map(|v| v * v).sum();
    if var < 1e-12 {
        return None;
    }
    let mut decayed = false;
    let mut best: Option<(usize, f64)> = None;
    for lag in 1..n / 2 {
        let mut acc = 0.0;
        for i in 0..n - lag {
            acc += d[i] * d[i + lag];
        }
        let r = acc / var;
        if !decayed {
            if r < 0.3 {
                decayed = true;
            }
            continue;
        }
        if best.is_none_or(|(_, b)| r > b) {
            best = Some((lag, r));
        }
    }
    best
}

/// Cubic bezier progress at normalized time `t` for control points
/// (p1x, p1y, p2x, p2y), CSS `cubic-bezier` semantics: solve x(u) = t
/// for u by Newton + bisection fallback, then evaluate y(u).
fn bezier_progress(p: [f64; 4], t: f64) -> f64 {
    let (p1x, p1y, p2x, p2y) = (p[0], p[1], p[2], p[3]);
    let x = |u: f64| 3.0 * (1.0 - u).powi(2) * u * p1x + 3.0 * (1.0 - u) * u * u * p2x + u.powi(3);
    let dx = |u: f64| {
        3.0 * (1.0 - u).powi(2) * p1x + 6.0 * (1.0 - u) * u * (p2x - p1x)
            + 3.0 * u * u * (1.0 - p2x)
    };
    let mut u = t;
    for _ in 0..8 {
        let err = x(u) - t;
        let d = dx(u);
        if d.abs() < 1e-9 {
            break;
        }
        u -= err / d;
        u = u.clamp(0.0, 1.0);
    }
    if (x(u) - t).abs() > 1e-4 {
        // Newton wandered: bisect.
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..40 {
            u = (lo + hi) / 2.0;
            if x(u) < t {
                lo = u;
            } else {
                hi = u;
            }
        }
    }
    3.0 * (1.0 - u).powi(2) * u * p1y + 3.0 * (1.0 - u) * u * u * p2y + u.powi(3)
}

/// Fit cubic-bezier control points to a sampled progress curve
/// (progress[i] over uniform normalized time). Coarse grid search
/// refined twice: robust, dependency-free, and fast enough for ≤600
/// samples. Returns ([p1x, p1y, p2x, p2y], SSE).
fn fit_bezier(progress: &[f64]) -> ([f64; 4], f64) {
    let m = progress.len();
    let sse_of = |p: [f64; 4]| -> f64 {
        progress
            .iter()
            .enumerate()
            .map(|(i, &y)| {
                let t = i as f64 / (m - 1) as f64;
                (bezier_progress(p, t) - y).powi(2)
            })
            .sum()
    };
    let mut best = [0.25, 0.1, 0.25, 1.0];
    let mut best_sse = sse_of(best);
    let mut step = 0.25;
    for _ in 0..3 {
        let center = best;
        let grid = |c: f64, lo: f64, hi: f64| -> Vec<f64> {
            (-2i32..=2)
                .map(|k| (c + k as f64 * step).clamp(lo, hi))
                .collect()
        };
        for &p1x in &grid(center[0], 0.0, 1.0) {
            for &p1y in &grid(center[1], -0.5, 1.5) {
                for &p2x in &grid(center[2], 0.0, 1.0) {
                    for &p2y in &grid(center[3], -0.5, 1.5) {
                        let p = [p1x, p1y, p2x, p2y];
                        let sse = sse_of(p);
                        if sse < best_sse {
                            best_sse = sse;
                            best = p;
                        }
                    }
                }
            }
        }
        step /= 2.0;
    }
    (best, best_sse)
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f64 = 1000.0 / 60.0;

    fn track_json(xs: Vec<f64>, wrap: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "dt_ms": DT,
            "frames": xs.len(),
            "window_ms": xs.len() as f64 * DT,
            "tracks": [{
                "target": "ul.marquee", "property": "transform",
                "scroll_w": 6192, "scroll_h": 72,
                "xs": xs, "ys": vec![0.0; 150],
                "wrap": wrap,
            }],
        })
    }

    #[test]
    fn linear_velocity_from_clean_drift() {
        // 30 px/s leftward, like the stripe marquee.
        let xs: Vec<f64> = (0..150).map(|i| 100.0 - 0.03 * (i as f64) * DT).collect();
        let out = fit_payload(&track_json(xs, serde_json::Value::Null));
        assert_eq!(out.len(), 1);
        let m = &out[0];
        assert_eq!(m["model"], "linear");
        let v = m["velocity_px_s"].as_f64().unwrap();
        assert!((v + 30.0).abs() < 0.5, "velocity {v}");
        assert!(m["fit_r2"].as_f64().unwrap() > 0.99);
        assert_eq!(m["source"], "observed");
    }

    #[test]
    fn wrap_outliers_do_not_poison_velocity() {
        // Same drift, but the series wraps by +3096 px twice inside
        // the window (short-period sawtooth).
        let mut xs = vec![];
        let mut x = 100.0;
        for i in 0..150 {
            if i == 50 || i == 100 {
                x += 46.4; // wrap after ~46 px of travel
            }
            xs.push(x);
            x -= 0.03 * DT;
        }
        let out = fit_payload(&track_json(xs, serde_json::Value::Null));
        let m = &out[0];
        assert_eq!(m["model"], "linear");
        let v = m["velocity_px_s"].as_f64().unwrap();
        assert!((v + 30.0).abs() < 1.0, "velocity {v}");
        let period = m["period_s"].as_f64().unwrap();
        assert!((period - 50.0 * DT / 1000.0).abs() < 0.1, "period {period}");
    }

    #[test]
    fn wrap_hunt_yields_period() {
        // No wrap in the dense window; the hunt saw one 3096 px jump.
        let xs: Vec<f64> = (0..150).map(|i| 100.0 - 0.03 * (i as f64) * DT).collect();
        let wrap = serde_json::json!({
            "axis": "x",
            "wrap1": { "t_ms": 60_000.0, "jump": 3096.0 },
            "wrap2": null,
        });
        let out = fit_payload(&track_json(xs, wrap));
        let m = &out[0];
        let period = m["period_s"].as_f64().unwrap();
        // 3096 px / 30 px/s = 103.2 s
        assert!((period - 103.2).abs() < 2.0, "period {period}");
        assert!(m["phase_s"].as_f64().is_some());
    }

    #[test]
    fn oscillation_detected_by_autocorrelation() {
        // 1 Hz sine, 40 px amplitude: no drift, clear period.
        let xs: Vec<f64> = (0..150)
            .map(|i| 40.0 * (2.0 * std::f64::consts::PI * (i as f64) * DT / 1000.0).sin())
            .collect();
        let out = fit_payload(&track_json(xs, serde_json::Value::Null));
        let m = &out[0];
        assert_eq!(m["model"], "periodic");
        let period = m["period_s"].as_f64().unwrap();
        assert!((period - 1.0).abs() < 0.15, "period {period}");
    }

    #[test]
    fn one_shot_fits_bezier() {
        // 300 ms ease-in-out move of 200 px, padded with rest.
        let ease = [0.42, 0.0, 0.58, 1.0];
        let mut xs = vec![0.0; 20];
        let steps = 18; // ~300 ms at 60 fps
        for i in 0..=steps {
            let t = i as f64 / steps as f64;
            xs.push(200.0 * bezier_progress(ease, t));
        }
        xs.extend(vec![200.0; 20]);
        let out = fit_payload(&track_json(xs, serde_json::Value::Null));
        let m = &out[0];
        assert_eq!(m["model"], "bezier");
        assert!(m["fit_r2"].as_f64().unwrap() > 0.98);
        let d = m["duration_ms"].as_f64().unwrap();
        assert!((d - 300.0).abs() < 40.0, "duration {d}");
        assert_eq!(m["distance_px"].as_f64().unwrap(), 200.0);
    }

    #[test]
    fn static_elements_are_skipped() {
        let xs = vec![100.0; 150];
        let out = fit_payload(&track_json(xs, serde_json::Value::Null));
        assert!(out.is_empty());
    }
}
