//! Visual-verification primitives: motion spec, animation seek, pixel diff.
//!
//! These are the "measuring instrument" half of agent verification.
//! A screenshot lets an agent *look*; these let it *measure*:
//!
//! - [`motion`] reads the page's animation/transition inventory as
//!   numbers (durations, easings, keyframes) via the Web Animations
//!   API and CSSOM, so motion can be copied and compared exactly.
//! - [`seek`] freezes animation time (pause + set `currentTime`), so a
//!   screenshot of an animated page becomes deterministic and two
//!   pages can be compared at the same animation instant.
//! - [`diff`] captures two windows (or a window and a baseline PNG)
//!   and reports the percentage of matching pixels plus the bounding
//!   boxes of the worst mismatched regions, optionally writing a
//!   heatmap PNG. The numeric score is a convergence signal: an agent
//!   iterating on a copy watches it climb toward 100%.

use crate::automation::{self, Reply};
use crate::Daemon;
use gtk::gdk;
use gtk::gdk::prelude::TextureExt;
use gtk::gio;
use gtk::glib;
use hwatu_ipc::Response;
use std::cell::RefCell;
use std::rc::Rc;
use webkit6::prelude::WebViewExt;

// ---- motion spec ----------------------------------------------------

/// Everything the browser will admit about how the page moves, as
/// JSON. `document.getAnimations()` covers running/paused CSS
/// animations, CSS transitions and WAAPI animations (with resolved
/// keyframes); the CSSOM walk covers `@keyframes` rules and
/// declared-but-idle `transition`/`animation` shorthands that only
/// fire on interaction (hover transitions nobody hovered yet).
pub fn motion(daemon: &Rc<Daemon>, id: Option<u64>, timeout_ms: Option<u64>, reply: Reply) {
    motion_value(daemon, id, timeout_ms, reply)
}

/// The declared-inventory eval behind [`motion`], reusable by
/// [`crate::observe`] so `--observe` merges into the same shape.
pub fn motion_value(daemon: &Rc<Daemon>, id: Option<u64>, timeout_ms: Option<u64>, reply: Reply) {
    const JS: &str = r#"
const MAX = 200;
const sel = (el) => {
  if (!el || !el.tagName) return null;
  let s = el.tagName.toLowerCase();
  if (el.id) return s + '#' + el.id;
  if (el.classList && el.classList.length) s += '.' + [...el.classList].slice(0, 3).join('.');
  return s;
};
const animations = document.getAnimations().slice(0, MAX).map(a => {
  const out = { state: a.playState };
  if (a.animationName) out.name = a.animationName;           // CSSAnimation
  if (a.transitionProperty) out.property = a.transitionProperty; // CSSTransition
  const ef = a.effect;
  if (ef) {
    const t = ef.getTiming();
    out.duration_ms = t.duration;
    out.delay_ms = t.delay;
    out.easing = t.easing;
    out.iterations = t.iterations;
    out.direction = t.direction;
    out.fill = t.fill;
    if (ef.target) out.target = sel(ef.target);
    try {
      out.keyframes = ef.getKeyframes().map(k => {
        const kf = {};
        for (const [key, v] of Object.entries(k)) {
          if (key === 'composite' || key === 'computedOffset') continue;
          kf[key] = v;
        }
        return kf;
      });
    } catch (e) { /* cross-origin effect */ }
  }
  out.current_time_ms = a.currentTime;
  return out;
});
// CSSOM: @keyframes rules + elements with declared transitions that
// are idle right now (they only show up in getAnimations mid-flight).
const keyframes = {};
const kfSeen = new Set();
for (const sheet of document.styleSheets) {
  let rules;
  try { rules = sheet.cssRules; } catch (e) { continue; } // cross-origin
  const walk = (list) => {
    for (const r of list) {
      if (r.type === CSSRule.KEYFRAMES_RULE && !kfSeen.has(r.name)) {
        kfSeen.add(r.name);
        keyframes[r.name] = [...r.cssRules].map(k => ({ offset: k.keyText, style: k.style.cssText }));
      } else if (r.cssRules) {
        walk(r.cssRules);
      }
    }
  };
  walk(rules);
}
const declared = [];
const els = document.querySelectorAll('*');
for (const el of els) {
  if (declared.length >= MAX) break;
  const st = getComputedStyle(el);
  const tp = st.transitionProperty;
  const td = st.transitionDuration;
  const hasTransition = tp && tp !== 'all' || (td && td.split(',').some(d => parseFloat(d) > 0));
  const hasAnimation = st.animationName && st.animationName !== 'none';
  if (!hasTransition && !hasAnimation) continue;
  const d = { target: sel(el) };
  if (hasTransition) {
    d.transition = {
      property: tp, duration: td,
      timing: st.transitionTimingFunction, delay: st.transitionDelay,
    };
  }
  if (hasAnimation) {
    d.animation = {
      name: st.animationName, duration: st.animationDuration,
      timing: st.animationTimingFunction, delay: st.animationDelay,
      iterations: st.animationIterationCount, direction: st.animationDirection,
      fill: st.animationFillMode, state: st.animationPlayState,
    };
  }
  declared.push(d);
}
return { animations, keyframes, declared };"#;
    automation::eval(daemon, id, JS.to_string(), timeout_ms, reply);
}

// ---- seek -----------------------------------------------------------

/// Freeze/scrub/resume animation time. Pausing and pinning
/// `currentTime` makes the rendered frame a pure function of the seek
/// target, which is what turns animated pages back into diffable
/// stills (the standard trick behind every non-flaky visual test).
pub fn seek(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
    time_ms: Option<f64>,
    progress: Option<f64>,
    resume: bool,
    timeout_ms: Option<u64>,
    reply: Reply,
) {
    if !resume && time_ms.is_none() && progress.is_none() {
        return reply(Response::err(
            "pass one of --time-ms <ms>, --progress <0..1>, or --resume",
        ));
    }
    let js = format!(
        r#"
const timeMs = {time_ms};
const progress = {progress};
const resume = {resume};
const anims = document.getAnimations();
let touched = 0;
for (const a of anims) {{
  try {{
    if (resume) {{
      a.play();
      touched++;
      continue;
    }}
    a.pause();
    let t = timeMs;
    if (t === null) {{
      const timing = a.effect ? a.effect.getTiming() : null;
      const dur = timing && typeof timing.duration === 'number' ? timing.duration : 0;
      const delay = timing ? timing.delay : 0;
      t = delay + dur * progress;
    }}
    a.currentTime = t;
    touched++;
  }} catch (e) {{ /* e.g. infinite timeline */ }}
}}
return {{ animations: anims.length, touched, resumed: resume }};"#,
        time_ms = time_ms.map_or("null".into(), |v| v.to_string()),
        progress = progress.map_or("null".into(), |v| v.to_string()),
    );
    automation::eval(daemon, id, js, timeout_ms, reply);
}

// ---- resize ---------------------------------------------------------

/// Resize a window so the *page* sees `w x h` CSS pixels.
///
/// GTK widget allocation is in *logical* pixels and WebKitGTK maps
/// logical px 1:1 onto CSS px — the surface scale only converts
/// logical to *device* px. An earlier version multiplied the request
/// by the page's devicePixelRatio, double-applying the scale: under
/// dpr 2 a `resize 360x800` landed the page at 720 CSS px (the
/// gate-runner carried a W/dpr workaround for it). Allocate the CSS
/// size directly.
///
/// Trust nothing: the reply measures innerWidth/innerHeight from the
/// page, and if some backend does not map logical px 1:1 onto CSS px
/// the allocation is corrected once from the measured ratio and
/// re-verified, so the caller gets the size it asked for (or an
/// honest measurement of the miss).
pub fn resize(daemon: &Rc<Daemon>, id: Option<u64>, w: i32, h: i32, reply: Reply) {
    if !(1..=16384).contains(&w) || !(1..=16384).contains(&h) {
        return reply(Response::err(format!("bad viewport {w}x{h}")));
    }
    let win = match resolve_window(daemon, id) {
        Ok(win) => win,
        Err(resp) => return reply(*resp),
    };
    let daemon2 = daemon.clone();
    let win_id = win.id;
    win.resize_viewport(w, h);
    const MEASURE: &str =
        "return { css_width: innerWidth, css_height: innerHeight, dpr: window.devicePixelRatio }";
    automation::eval(
        daemon,
        Some(win_id),
        MEASURE.into(),
        Some(2000),
        Box::new(move |resp| {
            let measured = match &resp {
                Response::Ok { value: Some(v), .. } => (
                    v.get("css_width")
                        .and_then(|x| x.as_i64())
                        .map(|x| x as i32),
                    v.get("css_height")
                        .and_then(|x| x.as_i64())
                        .map(|x| x as i32),
                ),
                _ => (None, None),
            };
            let (cw, ch) = match measured {
                // Measurement failed (about:blank early in load):
                // return whatever the eval said; the allocation stands.
                (Some(cw), Some(ch)) => (cw, ch),
                _ => return reply(resp),
            };
            let (aw, ah) = (
                corrected_allocation(w, w, cw),
                corrected_allocation(h, h, ch),
            );
            match (aw, ah) {
                (None, None) => reply(resp), // exact: logical px == CSS px
                (aw, ah) => {
                    let win = match daemon2.windows.borrow().get(&win_id).cloned() {
                        Some(w) => w,
                        None => return reply(Response::err(format!("no window {win_id}"))),
                    };
                    win.resize_viewport(aw.unwrap_or(w), ah.unwrap_or(h));
                    // Re-verify: the reply always carries what the page
                    // actually sees, so the caller never has to trust us.
                    automation::eval(&daemon2, Some(win_id), MEASURE.into(), Some(2000), reply);
                }
            }
        }),
    );
}

/// One-shot correction for backends where widget logical px do not
/// map 1:1 onto CSS px: given what was requested (CSS px), what was
/// allocated (logical px), and what the page measured (CSS px),
/// return the allocation that should land the request, or `None` if
/// the measurement already matches. Pure so the double-apply bug
/// class is pinned by unit tests.
fn corrected_allocation(requested: i32, allocated: i32, measured: i32) -> Option<i32> {
    if measured == requested || measured <= 0 {
        return None;
    }
    let corrected =
        (f64::from(allocated) * f64::from(requested) / f64::from(measured)).round() as i32;
    Some(corrected.clamp(1, 16384))
}

/// Window by id, or the sole window. (Same contract as the ipc_server
/// sync path used before Resize became async.)
fn resolve_window(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
) -> Result<Rc<crate::window::BrowserWindow>, Box<Response>> {
    let windows = daemon.windows.borrow();
    match id {
        Some(id) => windows
            .get(&id)
            .cloned()
            .ok_or_else(|| Box::new(Response::err(format!("no window {id}")))),
        None => {
            if windows.len() == 1 {
                Ok(windows.values().next().cloned().expect("len checked"))
            } else {
                Err(Box::new(Response::err("pass --id (several windows open)")))
            }
        }
    }
}

// ---- pixel diff -----------------------------------------------------

/// RGBA pixels of one captured frame.
struct Frame {
    width: usize,
    height: usize,
    /// Tightly packed RGBA, `width * height * 4` bytes.
    rgba: Vec<u8>,
}

impl Frame {
    fn from_texture(texture: &gdk::Texture) -> Frame {
        let mut downloader = gdk::TextureDownloader::new(texture);
        downloader.set_format(gdk::MemoryFormat::R8g8b8a8);
        let (bytes, stride) = downloader.download_bytes();
        let width = texture.width() as usize;
        let height = texture.height() as usize;
        let row = width * 4;
        let mut rgba = Vec::with_capacity(row * height);
        for y in 0..height {
            let start = y * stride;
            rgba.extend_from_slice(&bytes[start..start + row]);
        }
        Frame {
            width,
            height,
            rgba,
        }
    }

    fn from_png(path: &str) -> Result<Frame, String> {
        let texture = gdk::Texture::from_filename(path)
            .map_err(|e| format!("cannot read baseline {path}: {e}"))?;
        Ok(Frame::from_texture(&texture))
    }
}

/// Compare two frames. Overlapping area is diffed pixel-by-pixel with
/// a per-channel tolerance; any area exclusive to one frame (size
/// mismatch) counts as fully different, because "the copy is 40px
/// taller" *is* a visual difference.
struct DiffResult {
    total: usize,
    mismatched: usize,
    /// Per-pixel mismatch mask over the union canvas, row-major.
    mask: Vec<bool>,
    union_w: usize,
    union_h: usize,
}

fn diff_frames(a: &Frame, b: &Frame, tolerance: u8) -> DiffResult {
    let union_w = a.width.max(b.width);
    let union_h = a.height.max(b.height);
    let total = union_w * union_h;
    let mut mask = vec![false; total];
    let mut mismatched = 0;
    let tol = tolerance as i16;
    for y in 0..union_h {
        for x in 0..union_w {
            let inside_a = x < a.width && y < a.height;
            let inside_b = x < b.width && y < b.height;
            let differs = match (inside_a, inside_b) {
                (true, true) => {
                    let ia = (y * a.width + x) * 4;
                    let ib = (y * b.width + x) * 4;
                    let pa = &a.rgba[ia..ia + 4];
                    let pb = &b.rgba[ib..ib + 4];
                    (0..4).any(|c| (pa[c] as i16 - pb[c] as i16).abs() > tol)
                }
                // Exclusive area: present in one frame only.
                _ => true,
            };
            if differs {
                mask[y * union_w + x] = true;
                mismatched += 1;
            }
        }
    }
    DiffResult {
        total,
        mismatched,
        mask,
        union_w,
        union_h,
    }
}

/// Coarse mismatch regions: the union canvas is cut into a grid of
/// cells; cells over a mismatch threshold are merged (greedy flood
/// fill over the cell grid) into bounding boxes, worst-first. Grid
/// granularity keeps this O(pixels) and the output small enough for
/// an agent to act on ("fix the header first").
fn mismatch_regions(diff: &DiffResult) -> Vec<serde_json::Value> {
    const CELL: usize = 32;
    const MAX_REGIONS: usize = 10;
    if diff.union_w == 0 || diff.union_h == 0 {
        return vec![];
    }
    let cols = diff.union_w.div_ceil(CELL);
    let rows = diff.union_h.div_ceil(CELL);
    // Mismatched pixel count per cell.
    let mut cells = vec![0usize; cols * rows];
    for y in 0..diff.union_h {
        for x in 0..diff.union_w {
            if diff.mask[y * diff.union_w + x] {
                cells[(y / CELL) * cols + (x / CELL)] += 1;
            }
        }
    }
    // A cell is "hot" if ≥5% of its pixels mismatch.
    let hot: Vec<bool> = cells.iter().map(|&c| c * 20 >= CELL * CELL).collect();
    let mut seen = vec![false; cols * rows];
    let mut regions: Vec<(usize, usize, usize, usize, usize)> = vec![]; // x0,y0,x1,y1,count
    for start in 0..cols * rows {
        if !hot[start] || seen[start] {
            continue;
        }
        // Flood fill over 4-connected hot cells.
        let (mut x0, mut y0, mut x1, mut y1, mut count) = (cols, rows, 0usize, 0usize, 0usize);
        let mut stack = vec![start];
        seen[start] = true;
        while let Some(i) = stack.pop() {
            let (cx, cy) = (i % cols, i / cols);
            x0 = x0.min(cx);
            y0 = y0.min(cy);
            x1 = x1.max(cx);
            y1 = y1.max(cy);
            count += cells[i];
            let mut push = |j: usize| {
                if hot[j] && !seen[j] {
                    seen[j] = true;
                    stack.push(j);
                }
            };
            if cx > 0 {
                push(i - 1);
            }
            if cx + 1 < cols {
                push(i + 1);
            }
            if cy > 0 {
                push(i - cols);
            }
            if cy + 1 < rows {
                push(i + cols);
            }
        }
        regions.push((x0, y0, x1, y1, count));
    }
    regions.sort_by_key(|r| std::cmp::Reverse(r.4));
    regions
        .into_iter()
        .take(MAX_REGIONS)
        .map(|(x0, y0, x1, y1, count)| {
            serde_json::json!({
                "x": x0 * CELL,
                "y": y0 * CELL,
                "w": ((x1 + 1) * CELL).min(diff.union_w) - x0 * CELL,
                "h": ((y1 + 1) * CELL).min(diff.union_h) - y0 * CELL,
                "mismatched_pixels": count,
            })
        })
        .collect()
}

/// Write a heatmap PNG: frame `a` dimmed to 1/3 brightness, mismatched
/// pixels painted red. The picture an agent (or human) opens to see
/// *where* the copy is wrong.
fn write_heatmap(a: &Frame, diff: &DiffResult, path: &str) -> Result<(), String> {
    let (w, h) = (diff.union_w, diff.union_h);
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let o = (y * w + x) * 4;
            if diff.mask[y * w + x] {
                out[o] = 0xff; // red
                out[o + 3] = 0xff;
            } else if x < a.width && y < a.height {
                let i = (y * a.width + x) * 4;
                out[o] = a.rgba[i] / 3;
                out[o + 1] = a.rgba[i + 1] / 3;
                out[o + 2] = a.rgba[i + 2] / 3;
                out[o + 3] = 0xff;
            } else {
                out[o + 3] = 0xff;
            }
        }
    }
    let bytes = glib::Bytes::from_owned(out);
    let texture = gdk::MemoryTexture::new(
        w as i32,
        h as i32,
        gdk::MemoryFormat::R8g8b8a8,
        &bytes,
        w * 4,
    );
    texture
        .save_to_png(path)
        .map_err(|e| format!("heatmap write to {path} failed: {e}"))
}

/// Capture a window's frame asynchronously, then hand it to `done`.
fn capture(daemon: &Rc<Daemon>, id: u64, full: bool, done: Box<dyn FnOnce(Result<Frame, String>)>) {
    let win = match daemon.windows.borrow().get(&id).cloned() {
        Some(w) => w,
        None => return done(Err(format!("no window {id}"))),
    };
    win.restore();
    win.ensure_viewport();
    let view = match win.live_webview() {
        Some(v) => v,
        None => return done(Err(format!("window {id} has no live webview"))),
    };
    let region = if full {
        webkit6::SnapshotRegion::FullDocument
    } else {
        webkit6::SnapshotRegion::Visible
    };
    view.snapshot(
        region,
        webkit6::SnapshotOptions::NONE,
        gio::Cancellable::NONE,
        move |result| match result {
            Ok(texture) => done(Ok(Frame::from_texture(&texture))),
            Err(e) => done(Err(format!("snapshot of window {id} failed: {e}"))),
        },
    );
}

/// Diff two windows (or one window against a baseline PNG): capture
/// both frames, compare, reply with a match score and region hints.
#[allow(clippy::too_many_arguments)]
pub fn diff(
    daemon: &Rc<Daemon>,
    id: u64,
    other: Option<u64>,
    baseline: Option<String>,
    tolerance: Option<u8>,
    heatmap: Option<String>,
    full: bool,
    reply: Reply,
) {
    let tolerance = tolerance.unwrap_or(8);
    if other.is_some() == baseline.is_some() {
        return reply(Response::err(
            "pass exactly one of --other <window id> or --baseline <png path>",
        ));
    }

    let finish = move |a: Frame, b: Frame, reply: Reply| {
        let result = diff_frames(&a, &b, tolerance);
        let match_percent = if result.total == 0 {
            100.0
        } else {
            100.0 * (result.total - result.mismatched) as f64 / result.total as f64
        };
        let regions = mismatch_regions(&result);
        let mut heatmap_path = None;
        if let Some(path) = heatmap {
            if let Err(e) = write_heatmap(&a, &result, &path) {
                return reply(Response::err(e));
            }
            heatmap_path = Some(path);
        }
        // The envelope: exactly what this score is a claim about, and
        // nothing more. A diff verifies one engine at one viewport at
        // one moment; scores quoted without their envelope get read as
        // "the page is pixel-perfect everywhere", which is how broken
        // responsive layouts sail through verification.
        let envelope = serde_json::json!({
            "engine": format!(
                "webkitgtk {}.{}.{}",
                webkit6::functions::major_version(),
                webkit6::functions::minor_version(),
                webkit6::functions::micro_version(),
            ),
            "viewport": { "width": a.width, "height": a.height },
            "region": if full { "full_document" } else { "visible" },
            "caveat": "score covers only this engine/viewport/frame; other widths, engines, and animation times are unverified",
        });
        let mut value = serde_json::json!({
            "match_percent": (match_percent * 100.0).round() / 100.0,
            "mismatched_pixels": result.mismatched,
            "total_pixels": result.total,
            "a": { "width": a.width, "height": a.height },
            "b": { "width": b.width, "height": b.height },
            "tolerance": tolerance,
            "regions": regions,
            "envelope": envelope,
        });
        if let Some(p) = heatmap_path {
            value["heatmap"] = serde_json::Value::String(p);
        }
        reply(Response::value(value));
    };

    match (other, baseline) {
        (None, Some(path)) => {
            capture(
                daemon,
                id,
                full,
                Box::new(move |a| match (a, Frame::from_png(&path)) {
                    (Ok(a), Ok(b)) => finish(a, b, reply),
                    (Err(e), _) | (_, Err(e)) => reply(Response::err(e)),
                }),
            );
        }
        (Some(other_id), None) => {
            // Capture sequentially: both captures run on the GTK main
            // loop anyway, and sequencing keeps the state machine flat.
            let daemon2 = daemon.clone();
            let first: Rc<RefCell<Option<Frame>>> = Rc::new(RefCell::new(None));
            capture(
                daemon,
                id,
                full,
                Box::new(move |a| {
                    let a = match a {
                        Ok(a) => a,
                        Err(e) => return reply(Response::err(e)),
                    };
                    first.borrow_mut().replace(a);
                    let first2 = first.clone();
                    capture(
                        &daemon2,
                        other_id,
                        full,
                        Box::new(move |b| {
                            let b = match b {
                                Ok(b) => b,
                                Err(e) => return reply(Response::err(e)),
                            };
                            let a = first2.borrow_mut().take().expect("first frame stored");
                            finish(a, b, reply);
                        }),
                    );
                }),
            );
        }
        _ => unreachable!("validated above"),
    }
}

#[cfg(test)]
mod tests {
    use super::corrected_allocation;

    /// The regression this pins: `resize WxH` must request the CSS
    /// size as-is (logical px == CSS px in WebKitGTK), never W*dpr.
    /// With the old double-apply, requesting 360 under dpr 2 landed
    /// the page at 720 CSS px; the correction path must then shrink
    /// the allocation, and must be a no-op when the page already
    /// measures exactly what was asked.
    #[test]
    fn exact_measurement_needs_no_correction() {
        assert_eq!(corrected_allocation(360, 360, 360), None);
        assert_eq!(corrected_allocation(1920, 1920, 1920), None);
    }

    #[test]
    fn double_applied_scale_is_corrected() {
        // dpr-2 double apply: asked 360, page saw 720 -> halve.
        assert_eq!(corrected_allocation(360, 360, 720), Some(180));
        // hypothetical backend where logical px = CSS px * 2 the other
        // way: asked 360, page saw 180 -> double.
        assert_eq!(corrected_allocation(360, 360, 180), Some(720));
    }

    #[test]
    fn degenerate_measurements_do_not_explode() {
        assert_eq!(corrected_allocation(360, 360, 0), None);
        assert_eq!(corrected_allocation(360, 360, -5), None);
        // Correction stays in the request's valid range.
        assert_eq!(corrected_allocation(16384, 16384, 1), Some(16384));
        assert_eq!(corrected_allocation(1, 1, 16384), Some(1));
    }
}
