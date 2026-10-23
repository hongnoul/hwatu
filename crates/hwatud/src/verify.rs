// SPDX-License-Identifier: MIT
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

/// Shared with every engine backend: `page-scripts/motion.js`.
pub const MOTION_JS: &str = include_str!("../../../page-scripts/motion.js");

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
    automation::eval(daemon, id, MOTION_JS.to_string(), timeout_ms, reply);
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

use hwatu_pixeldiff::Frame;

fn frame_from_texture(texture: &gdk::Texture) -> Frame {
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

fn frame_from_png(path: &str) -> Result<Frame, String> {
    let texture = gdk::Texture::from_filename(path)
        .map_err(|e| format!("cannot read baseline {path}: {e}"))?;
    Ok(frame_from_texture(&texture))
}

fn frame_from_base64(encoded: &str) -> Result<Frame, String> {
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
    Frame::from_png_bytes(&bytes).map_err(|e| format!("inline baseline: {e}"))
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
    let region = crate::vocab::SnapshotArea::from_full_page(full).to_webkit();
    view.snapshot(
        region,
        webkit6::SnapshotOptions::NONE,
        gio::Cancellable::NONE,
        move |result| match result {
            Ok(texture) => done(Ok(frame_from_texture(&texture))),
            Err(e) => done(Err(format!("snapshot of window {id} failed: {e}"))),
        },
    );
}

/// Compare two frames and build the structured diff report (shared
/// with the CLI through `hwatu-pixeldiff`), then write or inline the
/// heatmap. Shared by [`diff`] and by `check --baseline`.
fn diff_value(
    a: &Frame,
    b: &Frame,
    tolerance: u8,
    min_region_px: Option<u32>,
    heatmap: Option<String>,
    heatmap_data: bool,
    full: bool,
) -> Result<serde_json::Value, String> {
    let engine = format!(
        "webkitgtk {}.{}.{}",
        webkit6::functions::major_version(),
        webkit6::functions::minor_version(),
        webkit6::functions::micro_version(),
    );
    let (mut value, heatmap_png) = hwatu_pixeldiff::report(
        a,
        b,
        tolerance,
        min_region_px,
        heatmap.is_some() || heatmap_data,
        full,
        &engine,
    )?;
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

pub struct DiffOptions {
    pub tolerance: Option<u8>,
    /// Region significance floor in mismatched pixels (default
    /// [`hwatu_pixeldiff::DEFAULT_MIN_REGION_PX`]).
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
                let b = frame_from_png(&baseline)?;
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
                let b = frame_from_base64(&baseline_data)?;
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
                Box::new(move |a| match (a, frame_from_png(&path)) {
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
                Box::new(move |a| match (a, frame_from_base64(&encoded)) {
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
    use super::{corrected_allocation, diff_value, Frame};

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
