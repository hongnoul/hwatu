// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Justin Hong
//! Engine-independent pixel diff for `hwatu diff`.
//!
//! Compares two RGBA frames with a per-channel tolerance, clusters
//! mismatches into regions on a 32 px grid, and reports a match score,
//! the worst region, and how many regions clear a significance floor.
//! The GTK daemon diffs its own captures with this; the CLI uses it to
//! diff screenshots from engines whose daemon has no native `diff`.

/// RGBA pixels of one captured frame.
pub struct Frame {
    pub width: usize,
    pub height: usize,
    /// Tightly packed RGBA, `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

impl Frame {
    pub fn from_png_bytes(bytes: &[u8]) -> Result<Frame, String> {
        const MAX_RGBA_BYTES: usize = 256 * 1024 * 1024;
        let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder
            .read_info()
            .map_err(|error| format!("cannot decode PNG: {error}"))?;
        let info = reader.info();
        let width = info.width as usize;
        let height = info.height as usize;
        let rgba_size = width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(4))
            .filter(|size| *size <= MAX_RGBA_BYTES)
            .ok_or_else(|| "PNG dimensions are too large".to_string())?;
        let output_size = reader.output_buffer_size();
        if output_size > MAX_RGBA_BYTES {
            return Err("decoded PNG is too large".to_string());
        }
        let mut decoded = vec![0; output_size];
        let frame = reader
            .next_frame(&mut decoded)
            .map_err(|error| format!("cannot decode PNG: {error}"))?;
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
            return Err("decoded PNG has inconsistent dimensions".to_string());
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
pub struct DiffResult {
    total: usize,
    mismatched: usize,
    /// Per-pixel mismatch mask over the union canvas, row-major.
    mask: Vec<bool>,
    union_w: usize,
    union_h: usize,
}

pub fn diff_frames(a: &Frame, b: &Frame, tolerance: u8) -> Result<DiffResult, String> {
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
pub fn heatmap_png(a: &Frame, diff: &DiffResult) -> Result<Vec<u8>, String> {
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

/// Compare two frames and build the structured diff report: match
/// score, region hints, envelope. Returns the report and, when
/// `want_heatmap`, the heatmap PNG bytes; the caller decides whether to
/// write them to a path or return them inline.
pub fn report(
    a: &Frame,
    b: &Frame,
    tolerance: u8,
    min_region_px: Option<u32>,
    want_heatmap: bool,
    full: bool,
    engine: &str,
) -> Result<(serde_json::Value, Option<Vec<u8>>), String> {
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
    let heatmap_png = want_heatmap.then(|| heatmap_png(a, &result)).transpose()?;
    // The envelope: exactly what this score is a claim about, and
    // nothing more. A diff verifies one engine at one viewport at
    // one moment; scores quoted without their envelope get read as
    // "the page is pixel-perfect everywhere", which is how broken
    // responsive layouts sail through verification.
    let envelope = serde_json::json!({
        "engine": engine,
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
    Ok((value, heatmap_png))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let value = report(&base, &moved, 0, None, false, false, "test")
            .unwrap()
            .0;
        // Global score stays quotable...
        assert!(value["match_percent"].as_f64().unwrap() > 98.0, "{value}");
        // ...but the gate still trips.
        assert_eq!(value["significant_regions"], 1, "{value}");
        assert_eq!(value["min_region_px"], DEFAULT_MIN_REGION_PX);
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
        let value = report(&base, &noisy, 0, None, false, false, "test")
            .unwrap()
            .0;
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
        let default = report(&base, &small, 0, None, false, false, "test")
            .unwrap()
            .0;
        assert_eq!(default["significant_regions"], 0, "{default}");
        let tuned = report(&base, &small, 0, Some(500), false, false, "test")
            .unwrap()
            .0;
        assert_eq!(tuned["significant_regions"], 1, "{tuned}");
        assert_eq!(tuned["min_region_px"], 500);
    }
}
