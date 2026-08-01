// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! `hwatu clone`: one-shot faithful copy of a live page.
//!
//! Phase 0 (stills): productizes the `examples/clone/` pipeline that
//! reached a 100.0% average pixel match across 20 viewport positions
//! against stripe.com. The flow:
//!
//! 1. Open the URL in a headless window, wait for settle, resize to
//!    the capture viewport.
//! 2. Run the embedded `extract.js` in the page: sweep-prime lazy
//!    content and scroll reveals, harvest canvas frames, pause
//!    animations, then serialize the *rendered* DOM plus CSSOM text,
//!    an asset manifest, transition-state pins, and inner scroll
//!    positions.
//! 3. Blank canvases (WebGL without preserveDrawingBuffer) fall back
//!    to an engine-side screenshot crop.
//! 4. Materialize: download assets (curl, parallel), inline fonts as
//!    data URLs, rewrite URLs to local copies, re-inject canvas
//!    frames, media-scope the pins to the capture width, restore
//!    scroll positions, and write a self-contained `index.html`.
//! 5. Verify (default): open the clone in a second headless window
//!    and run `Diff` at several scroll offsets against the live
//!    window; write `report.json` with per-position scores so
//!    "faithful" is a measured claim.
//!
//! Known Phase 0 limits, stated honestly: animations are frozen at
//! the captured frame (a *still* clone), scripts do not survive (no
//! interactivity), and cross-origin iframes keep their live URLs.

use hwatu_ipc::{LoadStage, OpenMode, Request, Response};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The in-page capture script (see module docs). Embedded so `clone`
/// works from any directory with no sidecar files.
const EXTRACT_JS: &str = include_str!("../assets/extract.js");

const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 \
                  (KHTML, like Gecko) Version/17.0 Safari/605.1.15";

const SETTLE_AFTER_RESIZE_MS: u64 = 500;
const SETTLE_AFTER_SCROLL_MS: u64 = 400;
const SETTLE_CANVAS_VISIBLE_MS: u64 = 1_200;
const SETTLE_AFTER_STYLE_ISOLATION_MS: u64 = 300;
const SETTLE_MIN_FLOOR_MS: u64 = 32;

const USAGE: &str = "usage: hwatu clone <url> [--out <dir>] [--viewport <WxH>] \
     [--tolerance <0-255>] [--no-verify] [--keep] [--timeout-ms <ms>]";

pub fn run(args: &[String]) -> i32 {
    let opts = match Opts::parse(args) {
        Ok(o) => o,
        Err(msg) => {
            eprintln!("{msg}");
            return 2;
        }
    };
    match clone(&opts) {
        Ok(summary) => {
            println!("{summary}");
            0
        }
        Err(msg) => {
            eprintln!("hwatu: clone: {msg}");
            1
        }
    }
}

#[derive(Debug, PartialEq)]
struct Opts {
    url: String,
    out: PathBuf,
    viewport: (i32, i32),
    tolerance: Option<u8>,
    verify: bool,
    keep: bool,
    timeout_ms: u64,
}

impl Opts {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut url = None;
        let mut out = None;
        let mut viewport = (1920, 1080);
        let mut tolerance = None;
        let mut verify = true;
        let mut keep = false;
        let mut timeout_ms = 180_000u64;
        let mut it = args.iter();
        while let Some(a) = it.next() {
            match a.as_str() {
                "--out" => out = Some(PathBuf::from(it.next().ok_or(USAGE)?)),
                "--viewport" => {
                    let v = it.next().ok_or(USAGE)?;
                    let (w, h) = v.split_once(['x', 'X']).ok_or(USAGE)?;
                    viewport = (
                        w.trim().parse().map_err(|_| USAGE)?,
                        h.trim().parse().map_err(|_| USAGE)?,
                    );
                }
                "--tolerance" => {
                    tolerance = Some(it.next().and_then(|s| s.parse().ok()).ok_or(USAGE)?)
                }
                "--no-verify" => verify = false,
                "--keep" => keep = true,
                "--timeout-ms" => {
                    timeout_ms = it.next().and_then(|s| s.parse().ok()).ok_or(USAGE)?
                }
                other if url.is_none() && !other.starts_with('-') => url = Some(other.to_string()),
                other => return Err(format!("unknown argument {other:?}\n{USAGE}")),
            }
        }
        let url = url.ok_or(USAGE)?;
        // Bare domains navigate like the rest of the CLI.
        let url = if url.contains("://") {
            url
        } else {
            format!("https://{url}")
        };
        let out = out.unwrap_or_else(|| {
            let host = url
                .split("://")
                .nth(1)
                .unwrap_or(&url)
                .split(['/', '?'])
                .next()
                .unwrap_or("clone");
            PathBuf::from(format!("{host}-clone"))
        });
        Ok(Opts {
            url,
            out,
            viewport,
            tolerance,
            verify,
            keep,
            timeout_ms,
        })
    }
}

// ---- daemon roundtrips ------------------------------------------------

/// One request, one reply, one connection (the daemon protocol is
/// one-shot per connection for everything except Subscribe).
fn call(req: &Request) -> Result<Response, String> {
    let mut stream = crate::connect_or_spawn().map_err(|e| format!("cannot reach daemon: {e}"))?;
    let mut payload = serde_json::to_vec(req).expect("serialize request");
    payload.push(b'\n');
    stream
        .write_all(&payload)
        .map_err(|e| format!("write: {e}"))?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|e| format!("read: {e}"))?;
    serde_json::from_str::<Response>(line.trim()).map_err(|e| format!("bad response: {e}"))
}

/// Like [`call`], but unwraps `Response::Err` into this Err.
fn call_ok(req: &Request) -> Result<Response, String> {
    match call(req)? {
        r @ Response::Ok { .. } => Ok(r),
        Response::Err { message } => Err(message),
    }
}

fn eval(id: u64, js: &str, timeout_ms: u64) -> Result<serde_json::Value, String> {
    match call_ok(&Request::Eval {
        id: Some(id),
        js: js.to_string(),
        timeout_ms: Some(timeout_ms),
    })? {
        Response::Ok { value, .. } => Ok(value.unwrap_or(serde_json::Value::Null)),
        Response::Err { message } => Err(message),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SettleWait {
    floor: Duration,
    timeout: Duration,
}

impl SettleWait {
    fn new(floor_ms: u64, timeout_ms: u64) -> Self {
        let floor_ms = floor_ms.max(SETTLE_MIN_FLOOR_MS);
        let timeout_ms = timeout_ms.max(floor_ms);
        Self {
            floor: Duration::from_millis(floor_ms),
            timeout: Duration::from_millis(timeout_ms),
        }
    }

    fn eval_timeout_ms(self) -> u64 {
        self.timeout
            .saturating_add(Duration::from_millis(50))
            .as_millis()
            .min(u64::MAX as u128) as u64
    }
}

/// Best-effort visual settle without protocol changes.
///
/// The page-side part waits for two animation frames and a stable scrollY,
/// which is the cheap "a frame was produced" contract available through
/// existing eval. Headless WebKit views can be unmapped, and an unmapped view
/// may never deliver requestAnimationFrame. The daemon eval timeout plus this
/// function's native `Instant` floor/timeout keep the wait bounded and preserve
/// the old minimum pause semantics instead of hanging or failing the clone.
fn wait_visual_settle(id: u64, floor_ms: u64, timeout_ms: u64, context: &str) {
    let wait = SettleWait::new(floor_ms, timeout_ms);
    let started = Instant::now();
    let floor = wait.floor.as_millis();
    let timeout = wait.timeout.as_millis();
    let js = format!(
        r#"
        const floor = {floor};
        const timeout = {timeout};
        const start = performance.now();
        return await new Promise((resolve) => {{
            let lastY = Number.isFinite(scrollY) ? scrollY : 0;
            let stableFrames = 0;
            let done = false;
            const finish = (settled, reason) => {{
                if (done) return;
                done = true;
                resolve({{ settled, reason, scrollY: Number.isFinite(scrollY) ? scrollY : 0 }});
            }};
            const tick = () => {{
                const y = Number.isFinite(scrollY) ? scrollY : 0;
                stableFrames = Math.abs(y - lastY) < 0.5 ? stableFrames + 1 : 0;
                lastY = y;
                if (performance.now() - start >= floor && stableFrames >= 1) {{
                    finish(true, 'double-raf-scroll-stable');
                }} else {{
                    requestAnimationFrame(tick);
                }}
            }};
            setTimeout(() => finish(false, 'page-timeout'), timeout);
            requestAnimationFrame(() => requestAnimationFrame(tick));
        }});
        "#
    );
    match eval(id, &js, wait.eval_timeout_ms()) {
        Ok(_) => {}
        Err(e) if e.contains("timed out") => {
            eprintln!("clone: settle {context}: native timeout fallback after {timeout} ms");
        }
        Err(e) => {
            eprintln!("clone: settle {context}: {e}; continuing after bounded wait");
        }
    }
    let elapsed = started.elapsed();
    if elapsed < wait.floor {
        std::thread::sleep(wait.floor - elapsed);
    }
}

fn open_headless(url: &str, timeout_ms: u64) -> Result<u64, String> {
    let Response::Ok { window, .. } = call_ok(&Request::Open {
        url: Some(url.to_string()),
        app_id: None,
        mode: OpenMode::Headless,
    })?
    else {
        unreachable!("call_ok filters Err")
    };
    let id = window.ok_or("open returned no window")?.id;
    call_ok(&Request::WaitLoad {
        id: Some(id),
        until: LoadStage::Settled,
        timeout_ms: Some(timeout_ms),
    })?;
    Ok(id)
}

fn close(id: u64) {
    let _ = call(&Request::Close { id });
}

// ---- the pipeline -----------------------------------------------------

fn clone(opts: &Opts) -> Result<String, String> {
    std::fs::create_dir_all(opts.out.join("assets"))
        .map_err(|e| format!("cannot create {}: {e}", opts.out.display()))?;

    eprintln!("clone: opening {} headless...", opts.url);
    let live = open_headless(&opts.url, opts.timeout_ms)?;
    // Close on any failure: a leaked headless window is invisible and
    // therefore never manually cleaned up.
    let result = clone_with_window(opts, live);
    if result.is_err() || !opts.keep {
        close(live);
    }
    result
}

fn clone_with_window(opts: &Opts, live: u64) -> Result<String, String> {
    call_ok(&Request::Resize {
        id: Some(live),
        width: opts.viewport.0,
        height: opts.viewport.1,
    })?;

    eprintln!("clone: capturing rendered DOM (sweep + freeze)...");
    let cap = eval(live, EXTRACT_JS, opts.timeout_ms)?;
    if cap.get("html").and_then(|v| v.as_str()).is_none() {
        return Err(format!("capture returned no html: {cap}"));
    }
    std::fs::write(
        opts.out.join("capture.json"),
        serde_json::to_vec_pretty(&cap).expect("serialize capture"),
    )
    .map_err(|e| format!("write capture.json: {e}"))?;

    crop_blank_canvases(&cap, live, &opts.out)?;

    eprintln!("clone: materializing...");
    let stats = materialize(&cap, &opts.out)?;
    eprintln!(
        "clone: fetched {}/{} assets, index.html {} bytes",
        stats.fetched, stats.wanted, stats.html_bytes
    );

    let mut summary = format!(
        "clone written to {} ({}/{} assets)",
        opts.out.display(),
        stats.fetched,
        stats.wanted
    );

    if opts.verify {
        eprintln!("clone: verifying against the live page...");
        let report = verify(opts, live)?;
        std::fs::write(
            opts.out.join("report.json"),
            serde_json::to_vec_pretty(&report).expect("serialize report"),
        )
        .map_err(|e| format!("write report.json: {e}"))?;
        let avg = report
            .get("average_match_percent")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        summary.push_str(&format!(
            "\nverify: {avg:.1}% average pixel match ({} positions), report.json written",
            report
                .get("positions")
                .and_then(|v| v.as_array())
                .map(Vec::len)
                .unwrap_or(0)
        ));
    }
    Ok(summary)
}

// ---- blank-canvas fallback (engine screenshot crop) -------------------

/// WebGL canvases without `preserveDrawingBuffer` serialize to blank
/// data URLs; capture those from the engine instead: scroll the canvas
/// into view, isolate its paint, screenshot, and crop its rect.
fn crop_blank_canvases(cap: &serde_json::Value, live: u64, out: &Path) -> Result<(), String> {
    let canvases = cap.get("canvases").and_then(|v| v.as_array());
    let blanks: Vec<&serde_json::Value> = canvases
        .map(|cs| {
            cs.iter()
                .filter(|c| c.get("blank").and_then(|b| b.as_bool()) == Some(true))
                .collect()
        })
        .unwrap_or_default();
    if blanks.is_empty() {
        return Ok(());
    }
    let dpr = cap
        .pointer("/viewport/dpr")
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0);
    for c in blanks {
        let (i, w, h) = (
            c.get("i").and_then(|v| v.as_u64()).unwrap_or(0),
            c.get("w").and_then(|v| v.as_f64()).unwrap_or(0.0),
            c.get("h").and_then(|v| v.as_f64()).unwrap_or(0.0),
        );
        let doc_x = c.get("doc_x").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let doc_y = c.get("doc_y").and_then(|v| v.as_f64()).unwrap_or(0.0);
        if w < 1.0 || h < 1.0 {
            continue;
        }
        let scroll_y = (doc_y - 80.0).max(0.0);
        eval(live, &format!("scrollTo(0,{scroll_y}); return 0"), 10_000)?;
        // Give rAF draw loops a bounded chance to paint now that it is visible.
        wait_visual_settle(
            live,
            SETTLE_MIN_FLOOR_MS,
            SETTLE_CANVAS_VISIBLE_MS,
            "canvas-visible",
        );
        // Isolate the canvas paint: visibility on ancestors is
        // overridable by descendants, so the canvas stays visible while
        // overlaying DOM text does not get baked into the crop.
        let iso = format!(
            "const st=document.createElement('style'); st.id='hwatu-iso';\
             st.textContent='body * {{ visibility: hidden !important }} \
             [data-hwatu-canvas=\"{i}\"] {{ visibility: visible !important }}';\
             document.head.appendChild(st); return 1"
        );
        eval(live, &iso, 10_000)?;
        wait_visual_settle(
            live,
            SETTLE_MIN_FLOOR_MS,
            SETTLE_AFTER_STYLE_ISOLATION_MS,
            "canvas-isolated",
        );
        let shot = out.join(format!("canvas-shot-{i}.png"));
        let Response::Ok { path, .. } = call_ok(&Request::Screenshot {
            id: Some(live),
            path: Some(shot.to_string_lossy().into_owned()),
            full: false,
        })?
        else {
            unreachable!()
        };
        eval(
            live,
            "document.getElementById('hwatu-iso')?.remove(); return 1",
            10_000,
        )?;
        let shot_path = path.map(PathBuf::from).unwrap_or(shot);
        let dest = out.join("assets").join(format!("canvas-{i}.png"));
        let vx = (doc_x * dpr).round() as u32;
        let vy = ((doc_y - scroll_y) * dpr).round() as u32;
        let cw = (w * dpr).round() as u32;
        let ch = (h * dpr).round() as u32;
        match crop_png(&shot_path, vx, vy, cw, ch, &dest) {
            Ok(()) => eprintln!("clone: canvas {i}: engine crop {cw}x{ch}+{vx}+{vy}"),
            Err(e) => eprintln!("clone: canvas {i}: crop failed ({e}); left blank"),
        }
        let _ = std::fs::remove_file(&shot_path);
    }
    eval(live, "scrollTo(0,0); return 0", 10_000)?;
    Ok(())
}

/// Crop `src` to `w x h + x + y` and write `dest`. Pure Rust (png
/// crate); clamps the rect to the image bounds.
fn crop_png(src: &Path, x: u32, y: u32, w: u32, h: u32, dest: &Path) -> Result<(), String> {
    let file = std::fs::File::open(src).map_err(|e| format!("open {}: {e}", src.display()))?;
    let decoder = png::Decoder::new(BufReader::new(file));
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let bpp = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        other => return Err(format!("unsupported screenshot color type {other:?}")),
    };
    let (iw, ih) = (info.width, info.height);
    let x = x.min(iw.saturating_sub(1));
    let y = y.min(ih.saturating_sub(1));
    let w = w.min(iw - x);
    let h = h.min(ih - y);
    if w == 0 || h == 0 {
        return Err("empty crop rect".into());
    }
    let row = (iw as usize) * bpp;
    let mut out_buf = Vec::with_capacity((w as usize) * (h as usize) * bpp);
    for r in y..y + h {
        let start = (r as usize) * row + (x as usize) * bpp;
        out_buf.extend_from_slice(&buf[start..start + (w as usize) * bpp]);
    }
    let file =
        std::fs::File::create(dest).map_err(|e| format!("create {}: {e}", dest.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(info.color_type);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    writer
        .write_image_data(&out_buf)
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ---- materialize ------------------------------------------------------

struct MaterializeStats {
    fetched: usize,
    wanted: usize,
    html_bytes: usize,
}

fn materialize(cap: &serde_json::Value, out: &Path) -> Result<MaterializeStats, String> {
    let base = cap
        .get("base")
        .and_then(|v| v.as_str())
        .ok_or("capture has no base url")?
        .to_string();
    let mut html = cap
        .get("html")
        .and_then(|v| v.as_str())
        .ok_or("capture has no html")?
        .to_string();

    // 1. Ordered stylesheet list (cascade order matters). Entries are
    //    {base, text} (read via CSSOM) or {href} (cross-origin: fetch).
    let mut all_css: Vec<(String, String)> = Vec::new();
    if let Some(sheets) = cap.get("sheets").and_then(|v| v.as_array()) {
        for sh in sheets {
            if let Some(text) = sh.get("text").and_then(|v| v.as_str()) {
                let sheet_base = sh
                    .get("base")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&base)
                    .to_string();
                all_css.push((sheet_base, text.to_string()));
            } else if let Some(href) = sh.get("href").and_then(|v| v.as_str()) {
                match fetch(href) {
                    Some(body) => {
                        all_css.push((href.to_string(), String::from_utf8_lossy(&body).into()))
                    }
                    None => eprintln!("clone: miss stylesheet {href}"),
                }
            }
        }
    }

    // 2. Asset set: page manifest + every url(...) in the CSS,
    //    absolutized against each sheet's own URL.
    let mut assets: BTreeSet<String> = cap
        .get("assets")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    let all_css: Vec<(String, String)> = all_css
        .into_iter()
        .map(|(b, t)| {
            let abs = rewrite_css_urls(&t, |u| {
                if u.starts_with("data:") {
                    return u.to_string();
                }
                match url_join(&b, u) {
                    Some(a) => {
                        assets.insert(a.clone());
                        a
                    }
                    None => u.to_string(),
                }
            });
            (b, abs)
        })
        .collect();

    // 3. Download in parallel (curl keeps the client crate free of an
    //    HTTP stack, matching `hwatu update`). Fonts embed as data URLs
    //    so the real face exists at first paint: a swapping font
    //    invalidates text layers late, and WebKit blend-mode layers can
    //    keep the fallback paint, double-exposing headlines.
    let wanted = assets.len();
    let mapping = fetch_all(&assets, out);
    eprintln!("clone: fetched {}/{} assets", mapping.len(), wanted);

    let css_joined = all_css
        .iter()
        .map(|(_, t)| t.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let mut css = rewrite_to_local(&css_joined, &mapping, &base);
    html = rewrite_to_local(&html, &mapping, &base);

    // 4. Canvas freeze: swap each canvas for its captured frame (data
    //    URL, or the engine crop written by crop_blank_canvases).
    if let Some(canvases) = cap.get("canvases").and_then(|v| v.as_array()) {
        for c in canvases {
            let i = c.get("i").and_then(|v| v.as_u64()).unwrap_or(0);
            let w = c.get("w").and_then(|v| v.as_i64()).unwrap_or(0);
            let h = c.get("h").and_then(|v| v.as_i64()).unwrap_or(0);
            let crop_rel = format!("assets/canvas-{i}.png");
            let src = if c.get("blank").and_then(|v| v.as_bool()) == Some(true)
                && out.join(&crop_rel).exists()
            {
                Some(crop_rel)
            } else {
                c.get("data").and_then(|v| v.as_str()).map(String::from)
            };
            if let Some(src) = src {
                html = replace_canvas(&html, i, &src, w, h);
            }
        }
    }

    // 5. Media-scoped transition pins: bake this capture's rendered
    //    transition state, but only near the capture width so other
    //    widths stay on the site's own responsive CSS.
    if let Some(pins) = cap.get("pins").and_then(|v| v.as_array()) {
        if !pins.is_empty() {
            let vw = cap
                .pointer("/viewport/w")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            css.push_str(&pin_media_block(pins, vw));
        }
    }

    // 6. Scroll restoration + snap disable: scroll-snap in the clone
    //    can land scrollers on a different snap point than the
    //    captured frame.
    if let Some(scrolls) = cap.get("scrolls").and_then(|v| v.as_array()) {
        if !scrolls.is_empty() {
            let js = scroll_restore_script(scrolls);
            if let Some(pos) = html.find("</body>") {
                html.insert_str(pos, &js);
            } else {
                html.push_str(&js);
            }
        }
    }

    // 7. Fonts are inline data URLs (decode is local and fast), so
    //    font-display:block is free and prevents any fallback first
    //    paint (WebKit keeps stale fallback paint inside blend-mode
    //    layers).
    css = pin_font_display(&css);

    // 8. Inject the inlined CSS and write.
    let style_block = format!("<style>\n{css}\n</style>");
    if let Some(pos) = html.find("</head>") {
        html.insert_str(pos, &style_block);
    } else {
        html = format!("{style_block}{html}");
    }
    let html_bytes = html.len();
    std::fs::write(out.join("index.html"), &html).map_err(|e| format!("write index.html: {e}"))?;
    Ok(MaterializeStats {
        fetched: mapping.len(),
        wanted,
        html_bytes,
    })
}

/// Download every asset; return url -> local reference (relative path
/// or data URL for fonts).
fn fetch_all(assets: &BTreeSet<String>, out: &Path) -> BTreeMap<String, String> {
    let queue: Mutex<Vec<String>> = Mutex::new(assets.iter().cloned().collect());
    let mapping: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());
    let workers = 8;
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| loop {
                let url = match queue.lock().expect("queue lock").pop() {
                    Some(u) => u,
                    None => break,
                };
                let Some(body) = fetch(&url) else {
                    eprintln!("clone: miss {url}");
                    continue;
                };
                let name = local_name(&url);
                let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
                let font_mime = match ext {
                    "woff2" => Some("font/woff2"),
                    "woff" => Some("font/woff"),
                    "ttf" => Some("font/ttf"),
                    "otf" => Some("font/otf"),
                    _ => None,
                };
                let local = if let Some(mime) = font_mime {
                    format!("data:{mime};base64,{}", base64(&body))
                } else if std::fs::write(out.join("assets").join(&name), &body).is_ok() {
                    format!("assets/{name}")
                } else {
                    continue;
                };
                mapping.lock().expect("mapping lock").insert(url, local);
            });
        }
    });
    mapping.into_inner().expect("mapping lock")
}

fn fetch(url: &str) -> Option<Vec<u8>> {
    let out = Command::new("curl")
        .args(["-fsSL", "--max-time", "30", "-A", UA, "--", url])
        .output()
        .ok()?;
    if out.status.success() && !out.stdout.is_empty() {
        Some(out.stdout)
    } else {
        None
    }
}

// ---- pure text transforms (unit-tested) --------------------------------

/// Stable local filename: content-independent hash of the URL plus a
/// best-effort extension, so the same URL maps to the same file across
/// the html/css rewrite passes.
fn local_name(url: &str) -> String {
    // FNV-1a, 64-bit: tiny, deterministic across runs and toolchains.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in url.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .rsplit('/')
        .next()
        .unwrap_or("");
    let ext = path.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    let clean = ext.len() <= 7 && !ext.is_empty() && ext.bytes().all(|b| b.is_ascii_alphanumeric());
    if clean {
        format!("{h:016x}.{ext}")
    } else {
        format!("{h:016x}")
    }
}

/// Resolve `rel` against `base` (RFC 3986-lite: absolute URLs pass
/// through, `//host` keeps the base scheme, `/path` keeps the origin,
/// anything else joins onto the base directory).
fn url_join(base: &str, rel: &str) -> Option<String> {
    if rel.contains("://") {
        return Some(rel.to_string());
    }
    let (scheme, rest) = base.split_once("://")?;
    if let Some(pp) = rel.strip_prefix("//") {
        return Some(format!("{scheme}://{pp}"));
    }
    let (host, path) = match rest.split_once('/') {
        Some((h, p)) => (h, format!("/{p}")),
        None => (rest, "/".to_string()),
    };
    if rel.starts_with('/') {
        return Some(format!("{scheme}://{host}{rel}"));
    }
    // Strip query/fragment from the base path, then its last segment.
    let path = path.split(['?', '#']).next().unwrap_or("/");
    let dir = &path[..path.rfind('/').map(|i| i + 1).unwrap_or(1)];
    // Normalize ../ and ./ segments.
    let mut segs: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
    for seg in rel.split('/') {
        match seg {
            "." | "" => {}
            ".." => {
                segs.pop();
            }
            s => segs.push(s),
        }
    }
    let trailing = if rel.ends_with('/') { "/" } else { "" };
    Some(format!("{scheme}://{host}/{}{trailing}", segs.join("/")))
}

/// Rewrite every `url(...)` occurrence in a stylesheet through `f`.
fn rewrite_css_urls(css: &str, mut f: impl FnMut(&str) -> String) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(pos) = rest.find("url(") {
        out.push_str(&rest[..pos + 4]);
        rest = &rest[pos + 4..];
        let Some(end) = rest.find(')') else {
            break;
        };
        let raw = &rest[..end];
        let trimmed = raw.trim().trim_matches(['\'', '"']);
        if trimmed.is_empty() {
            out.push_str(raw);
        } else {
            out.push_str(&f(trimmed));
        }
        out.push(')');
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Point absolute asset URLs (and their root-relative spellings on the
/// page's own origin) at the local copies. Longest URL first so a
/// prefix URL never clobbers a longer one.
fn rewrite_to_local(text: &str, mapping: &BTreeMap<String, String>, base: &str) -> String {
    let mut entries: Vec<(&String, &String)> = mapping.iter().collect();
    entries.sort_by_key(|(u, _)| std::cmp::Reverse(u.len()));
    let base_origin = origin_of(base);
    let mut text = text.to_string();
    for (url, local) in entries {
        text = text.replace(url.as_str(), local);
        // Root-relative spelling on the page's own origin, as it may
        // appear in the original document.
        let Some(origin) = &base_origin else { continue };
        let Some(rel) = url.strip_prefix(origin.as_str()) else {
            continue;
        };
        if !rel.is_empty() && rel != "/" {
            text = text
                .replace(&format!("\"{rel}\""), &format!("\"{local}\""))
                .replace(&format!("'{rel}'"), &format!("'{local}'"))
                .replace(&format!("url({rel})"), &format!("url({local})"));
        }
    }
    text
}

/// `https://host` for an absolute URL.
fn origin_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    Some(format!("{scheme}://{host}"))
}

/// Swap `<canvas ... data-hwatu-canvas="i" ...></canvas>` for an
/// `<img>` with the CSS box pinned: an `<img>` otherwise sizes itself
/// by the data URL's intrinsic ratio, not the canvas's layout.
fn replace_canvas(html: &str, i: u64, src: &str, w: i64, h: i64) -> String {
    let marker = format!("data-hwatu-canvas=\"{i}\"");
    let Some(mpos) = html.find(&marker) else {
        return html.to_string();
    };
    let Some(start) = html[..mpos].rfind("<canvas") else {
        return html.to_string();
    };
    let Some(gt) = html[mpos..].find('>') else {
        return html.to_string();
    };
    let open_end = mpos + gt;
    let Some(close) = html[open_end..].find("</canvas>") else {
        return html.to_string();
    };
    let end = open_end + close + "</canvas>".len();
    let attrs = &html[start + "<canvas".len()..open_end];
    let style = if w > 0 {
        format!(" style=\"width:{w}px;height:{h}px\"")
    } else {
        String::new()
    };
    format!(
        "{}<img{attrs} src=\"{src}\"{style}>{}",
        &html[..start],
        &html[end..]
    )
}

/// Pins bake one width's rendered transition state; scope them to a
/// window around the capture width so every other width falls back to
/// the site's own responsive CSS.
fn pin_media_block(pins: &[serde_json::Value], vw: i64) -> String {
    let lo = (vw - 40).max(1);
    let hi = vw + 40;
    let rules = pins
        .iter()
        .filter_map(|p| {
            let i = p.get("i")?.as_u64()?;
            let css = p.get("css")?.as_str()?;
            Some(format!("[data-hwatu-pin=\"{i}\"] {{ {css} }}"))
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "\n@media (min-width: {lo}px) and (max-width: {hi}px) {{\n\
         /* transition-state pins from capture at {vw}px */\n{rules}\n}}\n"
    )
}

/// The only JS in the clone: restore recorded inner scroll positions
/// with snap disabled.
fn scroll_restore_script(scrolls: &[serde_json::Value]) -> String {
    let payload: Vec<serde_json::Value> = scrolls
        .iter()
        .filter_map(|s| {
            Some(serde_json::json!({
                "i": s.get("i")?.as_u64()?,
                "l": s.get("left")?.as_f64()?,
                "t": s.get("top")?.as_f64()?,
            }))
        })
        .collect();
    format!(
        "<script>for (const s of {}) {{\
         const el = document.querySelector('[data-hwatu-scroll=\"' + s.i + '\"]');\
         if (el) {{ el.style.scrollSnapType = 'none'; el.scrollLeft = s.l; el.scrollTop = s.t; }}\
         }}</script>",
        serde_json::Value::Array(payload)
    )
}

/// Replace swappable `font-display` values with `block` (fonts are
/// inline data URLs, so blocking is free and never paints a fallback).
fn pin_font_display(css: &str) -> String {
    let mut out = css.to_string();
    for v in ["swap", "auto", "fallback", "optional"] {
        for sp in ["", " "] {
            out = out.replace(&format!("font-display:{sp}{v}"), "font-display:block");
        }
    }
    out
}

/// Minimal base64 (standard alphabet, padded); avoids a dependency for
/// one call site.
fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = u32::from_be_bytes([0, b[0], b[1], b[2]]);
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

// ---- verify -----------------------------------------------------------

/// Open the clone next to the live window and diff at several scroll
/// offsets. The report names exactly what was measured (the envelope):
/// this viewport, these offsets, nothing else.
fn verify(opts: &Opts, live: u64) -> Result<serde_json::Value, String> {
    let index = std::fs::canonicalize(opts.out.join("index.html"))
        .map_err(|e| format!("canonicalize index.html: {e}"))?;
    let clone_url = format!("file://{}", index.display());
    let clone_win = open_headless(&clone_url, opts.timeout_ms)?;
    let result = verify_with(opts, live, clone_win);
    if !opts.keep || result.is_err() {
        close(clone_win);
    }
    result
}

fn verify_with(opts: &Opts, live: u64, clone_win: u64) -> Result<serde_json::Value, String> {
    for id in [live, clone_win] {
        call_ok(&Request::Resize {
            id: Some(id),
            width: opts.viewport.0,
            height: opts.viewport.1,
        })?;
        // Freeze animations so the diff compares stills, not phases.
        let _ = call(&Request::Seek {
            id: Some(id),
            time_ms: None,
            progress: Some(0.0),
            resume: false,
            timeout_ms: Some(10_000),
        });
    }
    wait_visual_settle(
        live,
        SETTLE_MIN_FLOOR_MS,
        SETTLE_AFTER_RESIZE_MS,
        "live-resize-seek",
    );
    wait_visual_settle(
        clone_win,
        SETTLE_MIN_FLOOR_MS,
        SETTLE_AFTER_RESIZE_MS,
        "clone-resize-seek",
    );

    let max_scroll = eval(
        live,
        "return Math.max(0, document.documentElement.scrollHeight - innerHeight)",
        10_000,
    )?
    .as_f64()
    .unwrap_or(0.0);

    let fractions = [0.0, 0.25, 0.5, 0.75, 1.0];
    let mut positions = Vec::new();
    let mut sum = 0.0;
    let mut n = 0usize;
    for frac in fractions {
        let y = (max_scroll * frac).round();
        for id in [live, clone_win] {
            eval(id, &format!("scrollTo(0,{y}); return 0"), 10_000)?;
        }
        wait_visual_settle(
            live,
            SETTLE_MIN_FLOOR_MS,
            SETTLE_AFTER_SCROLL_MS,
            "live-scroll",
        );
        wait_visual_settle(
            clone_win,
            SETTLE_MIN_FLOOR_MS,
            SETTLE_AFTER_SCROLL_MS,
            "clone-scroll",
        );
        let Response::Ok { value, .. } = call_ok(&Request::Diff {
            id: live,
            other: Some(clone_win),
            baseline: None,
            tolerance: opts.tolerance,
            heatmap: None,
            full: false,
            timeout_ms: Some(30_000),
        })?
        else {
            unreachable!()
        };
        let value = value.unwrap_or(serde_json::Value::Null);
        let pct = value
            .get("match_percent")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        sum += pct;
        n += 1;
        eprintln!("clone: verify y={y}: {pct:.1}% match");
        positions.push(serde_json::json!({
            "scroll_y": y,
            "match_percent": pct,
            "regions": value.get("regions").cloned().unwrap_or(serde_json::Value::Null),
        }));
    }
    let avg = if n == 0 { 0.0 } else { sum / n as f64 };
    Ok(serde_json::json!({
        "url": opts.url,
        "viewport": { "w": opts.viewport.0, "h": opts.viewport.1 },
        "tolerance": opts.tolerance.unwrap_or(8),
        "positions": positions,
        "average_match_percent": (avg * 100.0).round() / 100.0,
        "envelope": "still clone; scores cover exactly this viewport and these scroll offsets",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opts_parse_defaults() {
        let o = Opts::parse(&["stripe.com".to_string()]).unwrap();
        assert_eq!(o.url, "https://stripe.com");
        assert_eq!(o.out, PathBuf::from("stripe.com-clone"));
        assert_eq!(o.viewport, (1920, 1080));
        assert!(o.verify);
        assert!(!o.keep);
    }

    #[test]
    fn opts_parse_flags() {
        let args: Vec<String> = [
            "https://example.com/x",
            "--out",
            "/tmp/o",
            "--viewport",
            "800x600",
            "--tolerance",
            "12",
            "--no-verify",
            "--keep",
            "--timeout-ms",
            "9000",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let o = Opts::parse(&args).unwrap();
        assert_eq!(o.out, PathBuf::from("/tmp/o"));
        assert_eq!(o.viewport, (800, 600));
        assert_eq!(o.tolerance, Some(12));
        assert!(!o.verify);
        assert!(o.keep);
        assert_eq!(o.timeout_ms, 9000);
    }

    #[test]
    fn opts_reject_unknown() {
        assert!(Opts::parse(&["x.com".into(), "--bogus".into()]).is_err());
        assert!(Opts::parse(&[]).is_err());
    }

    #[test]
    fn settle_wait_preserves_minimum_floor() {
        let wait = SettleWait::new(0, 1);
        assert_eq!(wait.floor, Duration::from_millis(SETTLE_MIN_FLOOR_MS));
        assert_eq!(wait.timeout, Duration::from_millis(SETTLE_MIN_FLOOR_MS));
    }

    #[test]
    fn settle_wait_keeps_native_timeout_above_page_timeout() {
        let wait = SettleWait::new(50, 400);
        assert_eq!(wait.floor, Duration::from_millis(50));
        assert_eq!(wait.timeout, Duration::from_millis(400));
        assert!(wait.eval_timeout_ms() > wait.timeout.as_millis() as u64);
    }

    #[test]
    fn url_join_cases() {
        let b = "https://a.com/css/site.css?v=1";
        assert_eq!(
            url_join(b, "img/x.png").unwrap(),
            "https://a.com/css/img/x.png"
        );
        assert_eq!(url_join(b, "../f.woff2").unwrap(), "https://a.com/f.woff2");
        assert_eq!(url_join(b, "/root.png").unwrap(), "https://a.com/root.png");
        assert_eq!(
            url_join(b, "//cdn.com/y.js").unwrap(),
            "https://cdn.com/y.js"
        );
        assert_eq!(url_join(b, "https://z.com/q").unwrap(), "https://z.com/q");
    }

    #[test]
    fn css_url_rewrite() {
        let css = "a{background:url('x.png')}b{src:url( \"y.woff2\" )}c{d:url(data:foo)}";
        let out = rewrite_css_urls(css, |u| {
            if u.starts_with("data:") {
                u.to_string()
            } else {
                format!("LOCAL/{u}")
            }
        });
        assert!(out.contains("url(LOCAL/x.png)"));
        assert!(out.contains("url(LOCAL/y.woff2)"));
        assert!(out.contains("url(data:foo)"));
    }

    #[test]
    fn canvas_swap() {
        let html = r#"<div><canvas class="c" data-hwatu-canvas="3" width="10"></canvas></div>"#;
        let out = replace_canvas(html, 3, "assets/canvas-3.png", 100, 50);
        assert!(out.contains(r#"<img class="c" data-hwatu-canvas="3" width="10" src="assets/canvas-3.png" style="width:100px;height:50px">"#), "{out}");
        assert!(!out.contains("<canvas"));
    }

    #[test]
    fn base64_matches_reference() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn font_display_pinned() {
        assert_eq!(
            pin_font_display("x{font-display: swap}y{font-display:auto}"),
            "x{font-display:block}y{font-display:block}"
        );
    }

    #[test]
    fn rewrite_local_root_relative() {
        let mut m = BTreeMap::new();
        m.insert(
            "https://a.com/img/x.png".to_string(),
            "assets/1.png".to_string(),
        );
        let text = r#"<img src="/img/x.png"> url(/img/x.png) <a href="https://a.com/img/x.png">"#;
        let out = rewrite_to_local(text, &m, "https://a.com/page");
        assert!(out.contains(r#"src="assets/1.png""#), "{out}");
        assert!(out.contains("url(assets/1.png)"), "{out}");
        assert!(out.contains(r#"href="assets/1.png""#), "{out}");
    }

    #[test]
    fn pin_block_scoped_to_width() {
        let pins = vec![serde_json::json!({"i": 0, "css": "opacity: 1 !important"})];
        let block = pin_media_block(&pins, 819);
        assert!(block.contains("min-width: 779px"));
        assert!(block.contains("max-width: 859px"));
        assert!(block.contains(r#"[data-hwatu-pin="0"] { opacity: 1 !important }"#));
    }
}
