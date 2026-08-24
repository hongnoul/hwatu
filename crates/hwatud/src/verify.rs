// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
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
/// logical px 1:1 onto CSS px; the surface scale only converts
/// logical to *device* px. An earlier version multiplied the request
/// by the page's devicePixelRatio, double-applying the scale: under
/// dpr 2 a `resize 360x800` landed the page at 720 CSS px (the
/// gate-runner carried a W/dpr workaround for it). Allocate the CSS
/// size directly.
///
/// Trust nothing: the reply measures innerWidth/innerHeight from the
/// page, and if some backend does not map logical px 1:1 onto CSS px
/// the allocation is corrected from the measurement and re-verified,
/// so the caller gets the size it asked for (or an honest measurement
/// of the miss).
///
/// Two backend behaviours have to be modelled, hence the small
/// correction *loop* rather than one shot:
///
/// - multiplicative: the backend scales logical px (dpr double-apply);
///   one ratio correction lands it.
/// - affine: the compositor/window chrome eats a fixed number of rows,
///   so measured = allocated - k. Seen under the display-free child
///   compositor (cage), where `resize 1920x1080` landed 1920x1081 and
///   360x640 landed 360x642 (issue #7). A ratio correction can never
///   converge on a constant offset, so after the first (ratio) attempt
///   the loop switches to offset correction: allocate
///   `allocated + (requested - measured)`. That converges in one more
///   pass for a constant k, and the loop is capped either way so a
///   pathological backend costs a bounded number of evals.
pub fn resize(daemon: &Rc<Daemon>, id: Option<u64>, w: i32, h: i32, reply: Reply) {
    if !(1..=16384).contains(&w) || !(1..=16384).contains(&h) {
        return reply(Response::err(format!("bad viewport {w}x{h}")));
    }
    let win = match resolve_window(daemon, id) {
        Ok(win) => win,
        Err(resp) => return reply(*resp),
    };
    let win_id = win.id;
    win.resize_viewport(w, h);
    measure_and_correct(
        daemon,
        ResizeAttempt {
            win_id,
            requested: (w, h),
            allocated: (w, h),
            attempt: 0,
        },
        reply,
    );
}

/// Maximum correction passes after the initial allocation. Pass 1 is
/// the ratio correction (dpr-style scaling), pass 2 the offset
/// correction (fixed chrome rows); a third is allowed for backends
/// whose offset itself shifts slightly with size. Bounded so a
/// backend that never converges costs a fixed number of evals and the
/// caller still gets an honest measurement.
const RESIZE_MAX_CORRECTIONS: u32 = 3;

/// A resize is cheap, but reading the resulting CSS viewport executes on
/// the page's main thread. Heavy pages can legitimately keep that thread
/// busy for more than the generic 2 s eval default during initial
/// hydration. Keep this bounded while giving verification commands enough
/// time to return the measured dimensions instead of a false timeout.
const RESIZE_MEASURE_TIMEOUT_MS: u64 = 10_000;

/// One step of the resize correction loop: which window, the CSS size
/// the caller asked for, the logical px currently allocated for it,
/// and how many corrections have already been made.
#[derive(Clone, Copy)]
struct ResizeAttempt {
    win_id: u64,
    requested: (i32, i32),
    allocated: (i32, i32),
    attempt: u32,
}

/// Measure the CSS viewport the page actually sees and, if it misses
/// the request, re-allocate and recurse.
fn measure_and_correct(daemon: &Rc<Daemon>, step: ResizeAttempt, reply: Reply) {
    const MEASURE: &str =
        "return { css_width: innerWidth, css_height: innerHeight, dpr: window.devicePixelRatio }";
    let ResizeAttempt {
        win_id,
        requested: (w, h),
        allocated: (alloc_w, alloc_h),
        attempt,
    } = step;
    let daemon2 = daemon.clone();
    automation::eval(
        daemon,
        Some(win_id),
        MEASURE.into(),
        Some(RESIZE_MEASURE_TIMEOUT_MS),
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
            if attempt >= RESIZE_MAX_CORRECTIONS {
                return reply(resp); // honest measurement of the miss
            }
            let (aw, ah) = (
                corrected_allocation(w, alloc_w, cw, attempt),
                corrected_allocation(h, alloc_h, ch, attempt),
            );
            match (aw, ah) {
                (None, None) => reply(resp), // exact: logical px == CSS px
                (aw, ah) => {
                    let win = match daemon2.windows.borrow().get(&win_id).cloned() {
                        Some(w) => w,
                        None => return reply(Response::err(format!("no window {win_id}"))),
                    };
                    let (next_w, next_h) = (aw.unwrap_or(alloc_w), ah.unwrap_or(alloc_h));
                    win.resize_viewport(next_w, next_h);
                    // Re-verify: the reply always carries what the page
                    // actually sees, so the caller never has to trust us.
                    measure_and_correct(
                        &daemon2,
                        ResizeAttempt {
                            allocated: (next_w, next_h),
                            attempt: attempt + 1,
                            ..step
                        },
                        reply,
                    );
                }
            }
        }),
    );
}

/// One-shot correction for backends where widget logical px do not
/// map 1:1 onto CSS px: given what was requested (CSS px), what was
/// allocated (logical px), what the page measured (CSS px), and which
/// correction `attempt` this is, return the allocation that should
/// land the request, or `None` if the measurement already matches.
///
/// `attempt == 0` assumes a multiplicative backend (dpr double-apply)
/// and scales; later attempts assume an affine one (fixed chrome rows
/// under the display-free child compositor) and shift by the residual.
/// Pure so both bug classes are pinned by unit tests.
fn corrected_allocation(
    requested: i32,
    allocated: i32,
    measured: i32,
    attempt: u32,
) -> Option<i32> {
    if measured == requested || measured <= 0 {
        return None;
    }
    let corrected = if attempt == 0 {
        (f64::from(allocated) * f64::from(requested) / f64::from(measured)).round() as i32
    } else {
        allocated.saturating_add(requested - measured)
    };
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

    fn from_base64(encoded: &str) -> Result<Frame, String> {
        let max_encoded = hwatu_ipc::INLINE_MAX_BYTES.div_ceil(3) * 4;
        if encoded.len() > max_encoded {
            return Err("inline baseline exceeds the encoded size limit".to_string());
        }
        let bytes = hwatu_ipc::base64::decode(encoded)
            .map_err(|error| format!("inline baseline is not valid base64: {error}"))?;
        if bytes.len() > hwatu_ipc::INLINE_MAX_BYTES {
            return Err(format!(
                "inline baseline is {} bytes; limit is {} bytes",
                bytes.len(),
                hwatu_ipc::INLINE_MAX_BYTES
            ));
        }
        Frame::from_png_bytes(&bytes)
    }

    fn from_png_bytes(bytes: &[u8]) -> Result<Frame, String> {
        const MAX_RGBA_BYTES: usize = 256 * 1024 * 1024;
        let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder
            .read_info()
            .map_err(|error| format!("cannot decode inline baseline PNG: {error}"))?;
        let info = reader.info();
        let width = info.width as usize;
        let height = info.height as usize;
        let rgba_size = width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(4))
            .filter(|size| *size <= MAX_RGBA_BYTES)
            .ok_or_else(|| "inline baseline dimensions are too large".to_string())?;
        let output_size = reader.output_buffer_size();
        if output_size > MAX_RGBA_BYTES {
            return Err("decoded inline baseline is too large".to_string());
        }
        let mut decoded = vec![0; output_size];
        let frame = reader
            .next_frame(&mut decoded)
            .map_err(|error| format!("cannot decode inline baseline PNG: {error}"))?;
        let data = &decoded[..frame.buffer_size()];
        let mut rgba = Vec::with_capacity(rgba_size);
        match frame.color_type {
            png::ColorType::Rgba => rgba.extend_from_slice(data),
            png::ColorType::Rgb => {
                for pixel in data.chunks_exact(3) {
                    rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
                }
            }
            png::ColorType::GrayscaleAlpha => {
                for pixel in data.chunks_exact(2) {
                    rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]);
                }
            }
            png::ColorType::Grayscale => {
                for value in data {
                    rgba.extend_from_slice(&[*value, *value, *value, 255]);
                }
            }
            png::ColorType::Indexed => {
                return Err("PNG palette expansion unexpectedly remained indexed".to_string());
            }
        }
        if rgba.len() != rgba_size {
            return Err("decoded inline baseline has inconsistent dimensions".to_string());
        }
        Ok(Frame {
            width,
            height,
            rgba,
        })
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

fn diff_frames(a: &Frame, b: &Frame, tolerance: u8) -> Result<DiffResult, String> {
    const MAX_DIFF_PIXELS: usize = 64 * 1024 * 1024;
    let union_w = a.width.max(b.width);
    let union_h = a.height.max(b.height);
    let total = union_w
        .checked_mul(union_h)
        .filter(|pixels| *pixels <= MAX_DIFF_PIXELS)
        .ok_or_else(|| {
            format!(
                "diff union canvas {union_w}x{union_h} exceeds the {MAX_DIFF_PIXELS}-pixel limit"
            )
        })?;
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
    Ok(DiffResult {
        total,
        mismatched,
        mask,
        union_w,
        union_h,
    })
}

/// Grid cell edge for region clustering (pixels).
const CELL: usize = 32;
/// Most regions reported in a diff reply.
const MAX_REGIONS: usize = 10;
/// Default significance floor: a region "counts" once it holds at
/// least this many mismatched pixels. One fully mismatched grid cell
/// (32x32). Separate from `tolerance` on purpose: tolerance decides
/// whether a *pixel* differs (AA noise), this decides whether a
/// *cluster* of differing pixels is big enough to gate on. Keeping
/// the two knobs apart is what makes a region threshold explainable
/// in review.
pub const DEFAULT_MIN_REGION_PX: u32 = 1024;

/// Coarse mismatch regions: the union canvas is cut into a grid of
/// cells; cells over a mismatch threshold are merged (greedy flood
/// fill over the cell grid) into bounding boxes, worst-first. Grid
/// granularity keeps this O(pixels) and the output small enough for
/// an agent to act on ("fix the header first"). Returns *all*
/// regions (cell coords + mismatch count) so the significance
/// summary is computed over the full set; the reply truncates.
fn mismatch_regions(diff: &DiffResult) -> Vec<(usize, usize, usize, usize, usize)> {
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
}

/// One region in pixel coordinates, with its share of mismatched
/// pixels. `density` (mismatched / bounding-box area) is what
/// separates "a moved button" (dense) from "spread AA noise" (thin)
/// when the global percentage looks equally good for both.
fn region_value(
    diff: &DiffResult,
    region: (usize, usize, usize, usize, usize),
) -> serde_json::Value {
    let (x0, y0, x1, y1, count) = region;
    let w = ((x1 + 1) * CELL).min(diff.union_w) - x0 * CELL;
    let h = ((y1 + 1) * CELL).min(diff.union_h) - y0 * CELL;
    let area = (w * h).max(1);
    serde_json::json!({
        "x": x0 * CELL,
        "y": y0 * CELL,
        "w": w,
        "h": h,
        "mismatched_pixels": count,
        "density": (count as f64 / area as f64 * 10000.0).round() / 10000.0,
    })
}

/// Write a heatmap PNG: frame `a` dimmed to 1/3 brightness, mismatched
/// pixels painted red. The picture an agent (or human) opens to see
/// *where* the copy is wrong.
fn heatmap_png(a: &Frame, diff: &DiffResult) -> Result<Vec<u8>, String> {
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
    let mut png = Vec::with_capacity(out.len() / 2 + 64);
    let mut encoder = png::Encoder::new(&mut png, w as u32, h as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    encoder.set_filter(png::FilterType::Sub);
    let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
    writer
        .write_image_data(&out)
        .map_err(|error| error.to_string())?;
    writer.finish().map_err(|error| error.to_string())?;
    Ok(png)
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

/// Compare two frames and build the structured diff report: match
/// score, region hints, envelope, optional heatmap. Shared by
/// [`diff`] and by `check --baseline` (which folds the pixel tier
/// into the one-roundtrip check).
fn diff_value(
    a: &Frame,
    b: &Frame,
    tolerance: u8,
    min_region_px: Option<u32>,
    heatmap: Option<String>,
    heatmap_data: bool,
    full: bool,
) -> Result<serde_json::Value, String> {
    let result = diff_frames(a, b, tolerance)?;
    let match_percent = if result.total == 0 {
        100.0
    } else {
        100.0 * (result.total - result.mismatched) as f64 / result.total as f64
    };
    let all_regions = mismatch_regions(&result);
    // Significance summary: the mean percentage is trivially gamed by
    // large unchanged backgrounds (a moved button still reads 99%+),
    // so gate on the *worst cluster* instead. `worst_region` is the
    // densest concentration of mismatch; `significant_regions` counts
    // clusters at or above the area floor. `match_percent` stays for
    // trend reporting; a gate of `significant_regions == 0` cannot be
    // bought with empty background.
    let min_region_px = min_region_px.unwrap_or(DEFAULT_MIN_REGION_PX) as usize;
    let significant = all_regions.iter().filter(|r| r.4 >= min_region_px).count();
    let worst_region = all_regions.first().map(|&r| region_value(&result, r));
    let regions: Vec<serde_json::Value> = all_regions
        .iter()
        .take(MAX_REGIONS)
        .map(|&r| region_value(&result, r))
        .collect();
    let heatmap_png = (heatmap.is_some() || heatmap_data)
        .then(|| heatmap_png(a, &result))
        .transpose()?;
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
        "min_region_px": min_region_px,
        "significant_regions": significant,
        "regions": regions,
        "envelope": envelope,
    });
    if let Some(worst) = worst_region {
        value["worst_region"] = worst;
    }
    if let (Some(path), Some(png)) = (heatmap, heatmap_png.as_ref()) {
        std::fs::write(&path, png)
            .map_err(|error| format!("heatmap write to {path} failed: {error}"))?;
        value["heatmap"] = serde_json::Value::String(path);
    }
    if heatmap_data {
        let png = heatmap_png.expect("heatmap requested above");
        if png.len() > hwatu_ipc::INLINE_MAX_BYTES {
            return Err(format!(
                "encoded heatmap is {} bytes; inline limit is {} bytes",
                png.len(),
                hwatu_ipc::INLINE_MAX_BYTES
            ));
        }
        value["heatmap_data"] = serde_json::Value::String(hwatu_ipc::base64::encode(&png));
    }
    Ok(value)
}

/// Diff window `id` against a baseline PNG and hand the structured
/// diff JSON to `done`. The callback shape (instead of a `Reply`)
/// lets `check` embed the diff as one field of its combined reply.
pub struct DiffOptions {
    pub tolerance: Option<u8>,
    /// Region significance floor in mismatched pixels (default
    /// [`DEFAULT_MIN_REGION_PX`]).
    pub min_region_px: Option<u32>,
    pub heatmap: Option<String>,
    pub heatmap_data: bool,
    pub full: bool,
}

pub fn diff_against_baseline(
    daemon: &Rc<Daemon>,
    id: u64,
    baseline: String,
    options: DiffOptions,
    done: Box<dyn FnOnce(Result<serde_json::Value, String>)>,
) {
    let tolerance = options.tolerance.unwrap_or(8);
    capture(
        daemon,
        id,
        options.full,
        Box::new(move |a| {
            let value = a.and_then(|a| {
                let b = Frame::from_png(&baseline)?;
                diff_value(
                    &a,
                    &b,
                    tolerance,
                    options.min_region_px,
                    options.heatmap,
                    options.heatmap_data,
                    options.full,
                )
            });
            done(value);
        }),
    );
}

pub fn diff_against_baseline_data(
    daemon: &Rc<Daemon>,
    id: u64,
    baseline_data: String,
    options: DiffOptions,
    done: Box<dyn FnOnce(Result<serde_json::Value, String>)>,
) {
    let tolerance = options.tolerance.unwrap_or(8);
    capture(
        daemon,
        id,
        options.full,
        Box::new(move |a| {
            let value = a.and_then(|a| {
                let b = Frame::from_base64(&baseline_data)?;
                diff_value(
                    &a,
                    &b,
                    tolerance,
                    options.min_region_px,
                    options.heatmap,
                    options.heatmap_data,
                    options.full,
                )
            });
            done(value);
        }),
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
    baseline_data: Option<String>,
    tolerance: Option<u8>,
    min_region_px: Option<u32>,
    heatmap: Option<String>,
    heatmap_data: bool,
    full: bool,
    reply: Reply,
) {
    let tolerance = tolerance.unwrap_or(8);
    let inputs = usize::from(other.is_some())
        + usize::from(baseline.is_some())
        + usize::from(baseline_data.is_some());
    if inputs != 1 {
        return reply(Response::err(
            "pass exactly one of --other, --baseline, or inline baseline data",
        ));
    }

    let finish = move |a: Frame, b: Frame, reply: Reply| match diff_value(
        &a,
        &b,
        tolerance,
        min_region_px,
        heatmap,
        heatmap_data,
        full,
    ) {
        Ok(value) => reply(Response::value(value)),
        Err(e) => reply(Response::err(e)),
    };

    match (other, baseline, baseline_data) {
        (None, Some(path), None) => {
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
        (None, None, Some(encoded)) => {
            capture(
                daemon,
                id,
                full,
                Box::new(move |a| match (a, Frame::from_base64(&encoded)) {
                    (Ok(a), Ok(b)) => finish(a, b, reply),
                    (Err(e), _) | (_, Err(e)) => reply(Response::err(e)),
                }),
            );
        }
        (Some(other_id), None, None) => {
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
    use super::{corrected_allocation, diff_frames, diff_value, Frame};

    #[test]
    fn extreme_aspect_ratios_cannot_expand_the_union_canvas() {
        let tall = Frame {
            width: 1,
            height: 10_000,
            rgba: vec![0; 10_000 * 4],
        };
        let wide = Frame {
            width: 10_000,
            height: 1,
            rgba: vec![0; 10_000 * 4],
        };
        let Err(error) = diff_frames(&tall, &wide, 0) else {
            panic!("oversized union canvas was accepted");
        };
        assert!(error.contains("10000x10000"));
        assert!(error.contains("pixel limit"));
    }

    #[test]
    fn inline_png_decode_and_heatmap_encode_never_touch_temp_paths() {
        let mut baseline = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut baseline, 1, 1);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[1, 2, 3]).unwrap();
        }
        let decoded = Frame::from_png_bytes(&baseline).unwrap();
        assert_eq!((decoded.width, decoded.height), (1, 1));
        assert_eq!(decoded.rgba, [1, 2, 3, 255]);

        let actual = Frame {
            width: 1,
            height: 1,
            rgba: vec![255, 255, 255, 255],
        };
        let value = diff_value(&actual, &decoded, 0, None, None, true, false).unwrap();
        let heatmap = value["heatmap_data"].as_str().unwrap();
        let bytes = hwatu_ipc::base64::decode(heatmap).unwrap();
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    }

    /// The dev.to critique this pins: a mean similarity score is
    /// trivially gamed by large unchanged backgrounds. One moved
    /// button on a big page reads 99%+ globally, but it must surface
    /// as one dense significant region an agent/CI can gate on
    /// (`significant_regions > 0`), with `worst_region` naming it.
    #[test]
    fn dense_local_change_is_significant_despite_high_global_score() {
        const W: usize = 640;
        const H: usize = 640;
        let base = Frame {
            width: W,
            height: H,
            rgba: vec![255u8; W * H * 4],
        };
        // "Moved button": one 64x64 block of solid difference.
        let mut rgba = vec![255u8; W * H * 4];
        for y in 96..160 {
            for x in 96..160 {
                let i = (y * W + x) * 4;
                rgba[i] = 0;
                rgba[i + 1] = 0;
                rgba[i + 2] = 0;
            }
        }
        let moved = Frame {
            width: W,
            height: H,
            rgba,
        };
        let value = diff_value(&base, &moved, 0, None, None, false, false).unwrap();
        // Global score stays quotable...
        assert!(value["match_percent"].as_f64().unwrap() > 98.0, "{value}");
        // ...but the gate still trips.
        assert_eq!(value["significant_regions"], 1, "{value}");
        assert_eq!(value["min_region_px"], super::DEFAULT_MIN_REGION_PX);
        let worst = &value["worst_region"];
        assert_eq!(worst["mismatched_pixels"], 64 * 64);
        assert!(
            worst["density"].as_f64().unwrap() > 0.9,
            "a solid block must be dense: {worst}"
        );
        // The worst region is also regions[0] (worst-first order).
        assert_eq!(value["regions"][0], *worst);
    }

    /// The other half of the same critique: thin, spread-out noise
    /// (antialiasing-style) must not trip the region gate, even when
    /// its pixel count rivals the moved button's. Sub-threshold cells
    /// never cluster, so `significant_regions` stays 0.
    #[test]
    fn spread_noise_is_not_significant() {
        const W: usize = 640;
        const H: usize = 640;
        let base = Frame {
            width: W,
            height: H,
            rgba: vec![255u8; W * H * 4],
        };
        // One differing pixel per 32x32 cell, everywhere: 400 pixels
        // of mismatch, but every cell stays under the 5% hot floor.
        let mut rgba = vec![255u8; W * H * 4];
        for cy in 0..(H / 32) {
            for cx in 0..(W / 32) {
                let i = ((cy * 32 + 16) * W + cx * 32 + 16) * 4;
                rgba[i] = 0;
            }
        }
        let noisy = Frame {
            width: W,
            height: H,
            rgba,
        };
        let value = diff_value(&base, &noisy, 0, None, None, false, false).unwrap();
        assert!(value["mismatched_pixels"].as_u64().unwrap() >= 400);
        assert_eq!(value["significant_regions"], 0, "{value}");
        assert!(value.get("worst_region").is_none(), "{value}");
    }

    /// The floor is a knob: lowering min_region_px makes smaller
    /// clusters count, so callers can tune what "significant" means
    /// without touching the pixel tolerance.
    #[test]
    fn min_region_px_knob_controls_significance() {
        const W: usize = 256;
        const H: usize = 256;
        let base = Frame {
            width: W,
            height: H,
            rgba: vec![255u8; W * H * 4],
        };
        // A 24x24 block: 576 mismatched pixels, dense within its
        // cells, but under the 1024 default floor.
        let mut rgba = vec![255u8; W * H * 4];
        for y in 32..56 {
            for x in 32..56 {
                let i = (y * W + x) * 4;
                rgba[i] = 0;
            }
        }
        let small = Frame {
            width: W,
            height: H,
            rgba,
        };
        let default = diff_value(&base, &small, 0, None, None, false, false).unwrap();
        assert_eq!(default["significant_regions"], 0, "{default}");
        let tuned = diff_value(&base, &small, 0, Some(500), None, false, false).unwrap();
        assert_eq!(tuned["significant_regions"], 1, "{tuned}");
        assert_eq!(tuned["min_region_px"], 500);
    }

    /// The regression this pins: `resize WxH` must request the CSS
    /// size as-is (logical px == CSS px in WebKitGTK), never W*dpr.
    /// With the old double-apply, requesting 360 under dpr 2 landed
    /// the page at 720 CSS px; the correction path must then shrink
    /// the allocation, and must be a no-op when the page already
    /// measures exactly what was asked.
    #[test]
    fn exact_measurement_needs_no_correction() {
        assert_eq!(corrected_allocation(360, 360, 360, 0), None);
        assert_eq!(corrected_allocation(1920, 1920, 1920, 0), None);
        // Also a no-op on later (offset-mode) attempts.
        assert_eq!(corrected_allocation(640, 677, 640, 1), None);
    }

    #[test]
    fn double_applied_scale_is_corrected() {
        // dpr-2 double apply: asked 360, page saw 720 -> halve.
        assert_eq!(corrected_allocation(360, 360, 720, 0), Some(180));
        // hypothetical backend where logical px = CSS px * 2 the other
        // way: asked 360, page saw 180 -> double.
        assert_eq!(corrected_allocation(360, 360, 180, 0), Some(720));
    }

    /// Issue #7: under the display-free child compositor the chrome
    /// eats a constant number of rows (measured = allocated - k), so
    /// the ratio correction overshoots forever. From attempt 1 the
    /// correction is additive and lands exactly in one more pass.
    #[test]
    fn constant_chrome_offset_is_corrected_additively() {
        // Observed on the VM: k = 37 rows.
        const K: i32 = 37;
        for (requested, allocated) in [(640, 640), (1080, 1080), (100, 100)] {
            let measured = allocated - K;
            let next = corrected_allocation(requested, allocated, measured, 1)
                .expect("miss must be corrected");
            assert_eq!(next, requested + K);
            // The re-measure of that allocation now matches exactly.
            assert_eq!(corrected_allocation(requested, next, next - K, 2), None);
        }
    }

    /// Why the loop switches modes rather than repeating the ratio
    /// correction: on an affine backend a ratio pass only ever removes
    /// *part* of a constant offset, so it never lands in one step (the
    /// pre-fix single-shot behaviour, i.e. issue #7), and how many
    /// steps it needs depends on rounding. Offset mode lands in one,
    /// for every size.
    #[test]
    fn offset_mode_lands_where_ratio_mode_needs_extra_passes() {
        const K: i32 = 37;
        for requested in [100, 360, 640, 1080, 1920] {
            // Ratio mode, one pass from the initial allocation: misses.
            let ratio_next = corrected_allocation(requested, requested, requested - K, 0)
                .expect("miss must be corrected");
            assert_ne!(
                ratio_next - K,
                requested,
                "ratio mode should not land {requested} in one pass"
            );
            // Offset mode, one pass: exact.
            let offset_next = corrected_allocation(requested, requested, requested - K, 1)
                .expect("miss must be corrected");
            assert_eq!(offset_next - K, requested);
        }
    }

    #[test]
    fn degenerate_measurements_do_not_explode() {
        assert_eq!(corrected_allocation(360, 360, 0, 0), None);
        assert_eq!(corrected_allocation(360, 360, -5, 1), None);
        // Correction stays in the request's valid range.
        assert_eq!(corrected_allocation(16384, 16384, 1, 0), Some(16384));
        assert_eq!(corrected_allocation(1, 1, 16384, 0), Some(1));
        // Offset mode clamps too: a huge negative shift floors at 1.
        assert_eq!(corrected_allocation(1, 1, 16384, 1), Some(1));
        assert_eq!(corrected_allocation(16384, 16384, 1, 1), Some(16384));
    }
}
