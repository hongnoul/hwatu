// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Justin Hong
//! Motion model fitting for `motion --observe`, shared by every engine.
//!
//! Input is the payload of `page-scripts/motion-observe.js` (per-frame
//! element positions sampled under the virtual clock). Output is one
//! model per moving element: linear velocity, loop period, oscillation
//! period, or a cubic-bezier easing, each with an r².

// ---- model fitting ---------------------------------------------------

/// One axis of one sampled element, ready to fit.
struct Series<'a> {
    positions: &'a [f64],
    /// Virtual ms between samples.
    dt_ms: f64,
}

/// Fitted model for one track, straight into output JSON.
pub fn fit_payload(payload: &serde_json::Value) -> Vec<serde_json::Value> {
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
    dedupe_fits(out)
}

/// Collapse entries whose fitted numbers are identical (a layout
/// shift moves every following sibling by the same curve): keep the
/// first, list the rest in `also_targets`. Distinct motion always has
/// distinct numbers, so real tracks never merge.
pub fn dedupe_fits(fits: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
    let mut out: Vec<serde_json::Value> = vec![];
    let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for fit in fits {
        let key = format!(
            "{}|{}|{}|{}|{}|{}",
            fit["model"],
            fit["axis"],
            fit["velocity_px_s"],
            fit["fit_r2"],
            fit["period_s"],
            fit["duration_ms"],
        );
        match index.get(&key) {
            Some(&i) => {
                let entry = out[i].as_object_mut().expect("fit is an object");
                let target = fit["target"].clone();
                match entry.get_mut("also_targets") {
                    Some(serde_json::Value::Array(a)) => a.push(target),
                    _ => {
                        entry.insert("also_targets".into(), serde_json::json!([target]));
                    }
                }
            }
            None => {
                index.insert(key, out.len());
                out.push(fit);
            }
        }
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
            let spans: Vec<f64> = jumps
                .windows(2)
                .map(|w| (w[1] - w[0]) as f64 * dt)
                .collect();
            map.insert(
                "period_s".into(),
                serde_json::json!(round2(median(&spans) / 1000.0)),
            );
        } else if let Some(w) = wrap {
            let wrap1_t = w["wrap1"]["t_ms"].as_f64();
            let wrap1_jump = w["wrap1"]["jump"].as_f64();
            let period_ms = match (wrap1_t, wrap1_jump) {
                (Some(_), Some(jump)) if slope.abs() > 1e-9 => Some((jump / slope).abs()),
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
        3.0 * (1.0 - u).powi(2) * p1x
            + 6.0 * (1.0 - u) * u * (p2x - p1x)
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
/// (`progress[i]` over uniform normalized time). Coarse grid search
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
