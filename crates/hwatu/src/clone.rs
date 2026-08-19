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
use std::io::BufReader;
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
    crate::write_request(&mut stream, req).map_err(|e| format!("write: {e}"))?;
    crate::read_response(&mut stream).map_err(|e| format!("read: {e}"))
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
        profile: None,
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
    if let Some(effects) = cap.get("scrollEffects").and_then(|v| v.as_array()) {
        if !effects.is_empty() {
            eprintln!(
                "clone: detected {} scroll-coupled text style region(s); see scroll-effects.json",
                effects.len()
            );
        }
    }

    crop_blank_canvases(&cap, live, &opts.out)?;

    // #44: the still clone drops script-driven motion (rAF loops, GSAP
    // tickers) and every event handler by design. Observe the live
    // window with the motion sampler and name what will not survive,
    // so "faithful" stays a measured claim instead of a still-envelope
    // technicality.
    eprintln!("clone: observing script-driven motion...");
    let (unreplicated_motion, motion_err) = observe_motion(live);
    if let Some(err) = &motion_err {
        eprintln!("clone: motion observe failed ({err}); reporting without motion tracks");
    }

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

    let stripped_scripts = cap.get("scripts").and_then(|v| v.as_u64()).unwrap_or(0);
    let interactive = cap.get("interactive").and_then(|v| v.as_u64()).unwrap_or(0);
    let replayed = stats
        .scroll_effects
        .iter()
        .filter(|e| e.get("replay").and_then(|v| v.as_str()) == Some("replayed"))
        .count();
    let honesty = format!(
        "still clone: {} script-driven motion track(s) and ~{} interactive element(s) not replicated",
        unreplicated_motion.len(),
        interactive
    );
    let mut report = serde_json::json!({
        "url": opts.url,
        "viewport": { "w": opts.viewport.0, "h": opts.viewport.1 },
        "unreplicated_motion": unreplicated_motion,
        "stripped_scripts": stripped_scripts,
        "interactive_elements": interactive,
        "parser_fixed_point": cap.get("parserFixedPoint").cloned().unwrap_or_else(|| serde_json::json!({
            "fixed_point": true,
            "rewritten": [],
            "reparsed_tag_count_deltas": [],
            "injected_css": false,
        })),
        "scroll_effects": stats.scroll_effects,
        "summary": honesty,
        "envelope": "still clone; scores cover exactly this viewport and these scroll offsets",
    });
    if let Some(err) = motion_err {
        report["motion_observe_error"] = serde_json::Value::String(err);
    }

    if opts.verify {
        eprintln!("clone: verifying against the live page...");
        let verified = verify(opts, live)?;
        if let (Some(map), Some(vmap)) = (report.as_object_mut(), verified.as_object()) {
            for k in ["tolerance", "positions", "average_match_percent"] {
                if let Some(v) = vmap.get(k) {
                    map.insert(k.to_string(), v.clone());
                }
            }
        }
        let avg = report
            .get("average_match_percent")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        summary.push_str(&format!(
            "\nverify: {avg:.1}% average pixel match ({} positions)",
            report
                .get("positions")
                .and_then(|v| v.as_array())
                .map(Vec::len)
                .unwrap_or(0)
        ));
    }
    std::fs::write(
        opts.out.join("report.json"),
        serde_json::to_vec_pretty(&report).expect("serialize report"),
    )
    .map_err(|e| format!("write report.json: {e}"))?;
    summary.push_str(&format!("\n{honesty}"));
    if !stats.scroll_effects.is_empty() {
        summary.push_str(&format!(
            "\nscroll effects: {replayed}/{} replayed by generated runtime",
            stats.scroll_effects.len()
        ));
    }
    summary.push_str("\nreport.json written");
    Ok(summary)
}

// ---- observed motion (issue #44) ---------------------------------------

/// Ask the daemon to observe script-driven motion in the live window
/// (`hwatu motion --observe` wiring) and translate the fitted tracks
/// into report entries. Failure is soft: an unobservable page still
/// clones, it just reports the error instead of tracks.
fn observe_motion(live: u64) -> (Vec<serde_json::Value>, Option<String>) {
    let resp = call(&Request::Motion {
        id: Some(live),
        observe: true,
        observe_ms: None,
        timeout_ms: Some(60_000),
    });
    match resp {
        Ok(Response::Ok { value: Some(v), .. }) => (unreplicated_motion_entries(&v), None),
        Ok(Response::Ok { .. }) => (vec![], Some("motion observe returned no data".into())),
        Ok(Response::Err { message }) => (vec![], Some(message)),
        Err(message) => (vec![], Some(message)),
    }
}

/// Map fitted observed-motion tracks to `unreplicated_motion` report
/// entries: `{selector, kind, period_ms, r2}` per track, with the
/// fitted model's honest r² carried through.
fn unreplicated_motion_entries(motion: &serde_json::Value) -> Vec<serde_json::Value> {
    let Some(observed) = motion.get("observed").and_then(|v| v.as_array()) else {
        return vec![];
    };
    observed
        .iter()
        .filter_map(|t| {
            let model = t.get("model")?.as_str()?;
            let kind = match model {
                "linear" if t.get("period_s").is_some() => "loop",
                "linear" => "linear",
                "periodic" => "oscillation",
                "bezier" => "one-shot",
                other => other,
            };
            let period_ms = t
                .get("period_s")
                .and_then(|v| v.as_f64())
                .map(|s| (s * 1000.0).round())
                .or_else(|| t.get("duration_ms").and_then(|v| v.as_f64()));
            Some(serde_json::json!({
                "selector": t.get("target").cloned().unwrap_or_default(),
                "kind": kind,
                "period_ms": period_ms,
                "r2": t.get("fit_r2").cloned().unwrap_or_default(),
            }))
        })
        .collect()
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
            data: false,
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
    /// Detected scroll-coupled effects annotated with replay status
    /// (`replayed` | `report-only`) and fit parameters when replayed.
    scroll_effects: Vec<serde_json::Value>,
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

    // 6.5. Scroll-coupled text-style replay (#45). These effects come
    // from JS scroll/rAF loops that mutate computed styles; the effect
    // is a pure function of scrollY, so for effects whose sampled
    // per-word flip thresholds fit a linear stagger model, emit a
    // generated replay runtime (same pattern as scroll_restore_script).
    // Below the fit gate, fall back to report-only evidence.
    let mut scroll_effects_out: Vec<serde_json::Value> = Vec::new();
    if let Some(effects) = cap.get("scrollEffects").and_then(|v| v.as_array()) {
        if !effects.is_empty() {
            let mut fits = Vec::new();
            let mut scroll_tracks = Vec::new();
            for effect in effects {
                let mut annotated = effect.clone();
                let entry = annotated.as_object_mut().expect("effect is an object");
                // The dense per-word sweep is fitting input, not report
                // material: it dwarfs the rest of the report.
                entry.remove("words");
                let kind = effect.get("kind").and_then(|v| v.as_str()).unwrap_or("");
                if matches!(
                    kind,
                    "scroll-coupled-visual-style" | "scroll-pin" | "scroll-triggered-time-style"
                ) {
                    if direct_track_supported(effect) {
                        entry.insert("replay".into(), serde_json::json!("replayed"));
                        entry.insert(
                            "replay_fit".into(),
                            serde_json::json!({ "model": "direct serialized scroll track runtime" }),
                        );
                        scroll_tracks.push(annotated.clone());
                    } else {
                        entry.insert("replay".into(), serde_json::json!("report-only"));
                    }
                } else {
                    match fit_scroll_effect(effect) {
                        Some(fit) => {
                            entry.insert("replay".into(), serde_json::json!("replayed"));
                            entry.insert("replay_fit".into(), fit.to_json());
                            fits.push(fit);
                        }
                        None => {
                            entry.insert("replay".into(), serde_json::json!("report-only"));
                        }
                    }
                }
                scroll_effects_out.push(annotated);
            }
            write_scroll_effects_report(out, &scroll_effects_out)?;
            let mut inject = String::new();
            if let Some(notice) = scroll_effect_notice(&scroll_effects_out) {
                inject.push_str(&notice);
            }
            if !fits.is_empty() {
                inject.push_str(&scroll_replay_script(&fits));
            }
            inject.push_str(&scroll_tracks_replay_script(&scroll_tracks));
            if !inject.is_empty() {
                if let Some(pos) = html.find("</body>") {
                    html.insert_str(pos, &inject);
                } else {
                    html.push_str(&inject);
                }
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
        scroll_effects: scroll_effects_out,
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

fn write_scroll_effects_report(out: &Path, effects: &[serde_json::Value]) -> Result<(), String> {
    let report = serde_json::json!({
        "envelope": "scroll-coupled text style mutation detected; effects marked `replayed` ship a generated scrollY->style runtime, `report-only` effects record representative states without replaying the scroll listener",
        "effects": effects,
    });
    std::fs::write(
        out.join("scroll-effects.json"),
        serde_json::to_vec_pretty(&report).expect("serialize scroll effects"),
    )
    .map_err(|e| format!("write scroll-effects.json: {e}"))
}

// ---- scroll-effect replay (issue #45) ----------------------------------

/// Fitted scrollY->style model for one detected scroll-coupled text
/// highlight:
///
/// ```text
/// progress = clamp01((innerHeight*A - rect.top) / (rect.height*B))
/// active_i = clamp01(progress * n_words - i)      // per-word stagger
/// style_i  = lerp(muted, highlighted, active_i)
/// ```
#[derive(Debug, Clone, PartialEq)]
struct ScrollEffectFit {
    selector: String,
    a: f64,
    b: f64,
    n_words: usize,
    r2: f64,
    muted_color: String,
    muted_opacity: String,
    lit_color: String,
    lit_opacity: String,
}

impl ScrollEffectFit {
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "model": "progress = clamp01((innerHeight*A - rect.top) / (rect.height*B)); active_i = clamp01(progress*n - i)",
            "a": round4(self.a),
            "b": round4(self.b),
            "n_words": self.n_words,
            "r2": round4(self.r2),
            "muted": { "color": self.muted_color, "opacity": self.muted_opacity },
            "highlighted": { "color": self.lit_color, "opacity": self.lit_opacity },
        })
    }
}

fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

/// The style axes the replay runtime writes.
fn word_style_key(state: &serde_json::Value) -> Option<String> {
    let color = state.get("color")?.as_str()?;
    let opacity = state.get("opacity")?.as_str()?;
    Some(format!("{color}|{opacity}"))
}

/// Minimum r² for the flip-threshold regression before a replay
/// runtime is emitted; below it the effect stays report-only.
const SCROLL_FIT_MIN_R2: f64 = 0.8;

/// Fit the scrollY->style model from the detector's fine per-word
/// sweep. Returns None (report-only fallback) when the data does not
/// support a confident fit: too few flipping words, non-monotone
/// flips, or a low-r² threshold regression.
fn fit_scroll_effect(effect: &serde_json::Value) -> Option<ScrollEffectFit> {
    let selector = effect.get("selector")?.as_str()?.to_string();
    let geom = effect.get("geometry")?;
    let doc_top = geom.get("docTop")?.as_f64()?;
    let height = geom.get("height")?.as_f64()?;
    let inner_height = geom.get("innerHeight")?.as_f64()?;
    if height <= 0.0 || inner_height <= 0.0 {
        return None;
    }
    let words = effect.get("words")?.as_array()?;
    let n = words.len();
    if n < 3 {
        return None;
    }

    // Per word: the scrollY where its (color, opacity) leaves the
    // entry state and stays away (a clean monotone flip).
    let mut flips: Vec<(usize, f64)> = Vec::new(); // (word index, flip y)
    let mut entry_styles: Vec<(&str, &str)> = Vec::new();
    let mut exit_styles: Vec<(&str, &str)> = Vec::new();
    for (k, word) in words.iter().enumerate() {
        let states = word.get("states").and_then(|v| v.as_array())?;
        if states.len() < 4 {
            continue;
        }
        let keys: Vec<String> = states.iter().filter_map(word_style_key).collect();
        if keys.len() != states.len() {
            continue;
        }
        let entry_key = &keys[0];
        let exit_key = &keys[keys.len() - 1];
        if entry_key == exit_key {
            continue; // never flipped in the sampled window
        }
        let flip = keys.iter().position(|key| key != entry_key)?;
        if flip == 0 {
            continue; // already flipped at entry: threshold unobserved
        }
        // Monotone: once away from the entry state, never back.
        if keys[flip..].iter().any(|key| key == entry_key) {
            continue;
        }
        let y_before = states[flip - 1].get("y")?.as_f64()?;
        let y_after = states[flip].get("y")?.as_f64()?;
        flips.push((k, (y_before + y_after) / 2.0));
        let first = &states[0];
        let last = &states[states.len() - 1];
        entry_styles.push((
            first.get("color")?.as_str()?,
            first.get("opacity")?.as_str()?,
        ));
        exit_styles.push((last.get("color")?.as_str()?, last.get("opacity")?.as_str()?));
    }
    if flips.len() < 3 || flips.len() * 2 < n {
        return None;
    }

    // Regress flip y against the word's stagger midpoint
    // t_k = (k + 0.5) / n:  y_k = C + D * t_k. The runtime's
    // active_i = clamp01(progress*n - i) crosses 0.5 exactly there, so
    // the fitted runtime reproduces the observed flip positions
    // regardless of how the source page parameterized its own model.
    let pts: Vec<(f64, f64)> = flips
        .iter()
        .map(|&(k, y)| ((k as f64 + 0.5) / n as f64, y))
        .collect();
    let (d, c, r2) = linear_regress(&pts)?;
    if r2 < SCROLL_FIT_MIN_R2 || d <= 0.0 {
        return None;
    }
    // y_k = docTop - innerHeight*A + t_k*height*B.
    let a = (doc_top - c) / inner_height;
    let b = d / height;
    if !a.is_finite() || !b.is_finite() || b <= 0.0 {
        return None;
    }

    let majority = |styles: &[(&str, &str)]| -> Option<(String, String)> {
        let mut counts: BTreeMap<(&str, &str), usize> = BTreeMap::new();
        for &s in styles {
            *counts.entry(s).or_insert(0) += 1;
        }
        counts
            .into_iter()
            .max_by_key(|&(_, c)| c)
            .map(|((color, op), _)| (color.to_string(), op.to_string()))
    };
    let (muted_color, muted_opacity) = majority(&entry_styles)?;
    let (lit_color, lit_opacity) = majority(&exit_styles)?;
    if muted_color == lit_color && muted_opacity == lit_opacity {
        return None;
    }

    Some(ScrollEffectFit {
        selector,
        a,
        b,
        n_words: n,
        r2,
        muted_color,
        muted_opacity,
        lit_color,
        lit_opacity,
    })
}

/// Least-squares fit y = intercept + slope * x. Returns
/// (slope, intercept, r²), or None for degenerate inputs.
fn linear_regress(pts: &[(f64, f64)]) -> Option<(f64, f64, f64)> {
    let n = pts.len() as f64;
    if pts.len() < 2 {
        return None;
    }
    let (mut sx, mut sy, mut sxx, mut sxy) = (0.0, 0.0, 0.0, 0.0);
    for &(x, y) in pts {
        sx += x;
        sy += y;
        sxx += x * x;
        sxy += x * y;
    }
    let denom = n * sxx - sx * sx;
    if denom.abs() < 1e-12 {
        return None;
    }
    let slope = (n * sxy - sx * sy) / denom;
    let intercept = (sy - slope * sx) / n;
    let mean = sy / n;
    let (mut sse, mut sst) = (0.0, 0.0);
    for &(x, y) in pts {
        sse += (y - (slope * x + intercept)).powi(2);
        sst += (y - mean).powi(2);
    }
    let r2 = if sst > 1e-12 { 1.0 - sse / sst } else { 1.0 };
    Some((slope, intercept, r2))
}

/// Generated replay runtime for fitted scroll-coupled highlights.
///
/// Hardened for hostile environments (lesson from debugging the
/// scale.com clone): never trust a single update signal. Headless and
/// virtual-clock contexts deliver neither native scroll events nor
/// rAF, so the driver stacks window + capture-phase document scroll
/// listeners, a rAF loop, AND a coarse setInterval fallback. The
/// update is idempotent and reads one getBoundingClientRect per
/// effect, so overdriving it is cheap.
fn scroll_replay_script(fits: &[ScrollEffectFit]) -> String {
    let payload: Vec<serde_json::Value> = fits
        .iter()
        .map(|f| {
            serde_json::json!({
                "sel": f.selector,
                "a": f.a,
                "b": f.b,
                "muted": { "color": f.muted_color, "opacity": f.muted_opacity },
                "lit": { "color": f.lit_color, "opacity": f.lit_opacity },
            })
        })
        .collect();
    format!(
        r#"<script id="hwatu-scroll-replay">(function () {{
  const cfg = {payload};
  const parse = (c) => {{
    const m = /rgba?\(([^)]+)\)/.exec(c);
    if (!m) return null;
    const p = m[1].split(',').map(parseFloat);
    return [p[0], p[1], p[2], p.length > 3 ? p[3] : 1];
  }};
  const lerp = (a, b, t) => a + (b - a) * t;
  const mix = (c1, c2, t) => 'rgba(' + Math.round(lerp(c1[0], c2[0], t)) + ',' +
    Math.round(lerp(c1[1], c2[1], t)) + ',' + Math.round(lerp(c1[2], c2[2], t)) + ',' +
    lerp(c1[3], c2[3], t).toFixed(3) + ')';
  const states = cfg.map((e) => {{
    const root = document.querySelector(e.sel);
    if (!root) return null;
    const words = [...root.querySelectorAll('[data-hwatu-scroll-word]')]
      .sort((x, y) => (+x.dataset.hwatuScrollWord) - (+y.dataset.hwatuScrollWord));
    if (!words.length) return null;
    return {{
      e, root, words,
      mc: parse(e.muted.color), lc: parse(e.lit.color),
      mo: parseFloat(e.muted.opacity), lo: parseFloat(e.lit.opacity),
    }};
  }}).filter(Boolean);
  const update = () => {{
    for (const s of states) {{
      const rect = s.root.getBoundingClientRect();
      const progress = Math.max(0, Math.min(1,
        (innerHeight * s.e.a - rect.top) / (rect.height * s.e.b)));
      const pos = progress * s.words.length;
      for (let i = 0; i < s.words.length; i++) {{
        const active = Math.max(0, Math.min(1, pos - i));
        const w = s.words[i];
        if (s.mc && s.lc) w.style.color = mix(s.mc, s.lc, active);
        if (Number.isFinite(s.mo) && Number.isFinite(s.lo)) {{
          w.style.opacity = String(lerp(s.mo, s.lo, active));
        }}
      }}
    }}
  }};
  addEventListener('scroll', update, {{ passive: true }});
  document.addEventListener('scroll', update, {{ passive: true, capture: true }});
  if (typeof requestAnimationFrame === 'function') {{
    const loop = () => {{ update(); requestAnimationFrame(loop); }};
    requestAnimationFrame(loop);
    // Smooth the discrete scroll-event steps like the live pages do, but
    // only once the frame clock provably advances: in headless/suspended
    // WebKit a CSSTransition freezes at time 0 and pins the computed
    // style to the START value forever, hiding every update.
    let frames = 0;
    const arm = (ts0) => requestAnimationFrame((ts1) => {{
      if (ts1 > ts0 && ++frames >= 2) {{
        for (const s of states) for (const w of s.words) {{
          w.style.transition = 'color 180ms ease-out, opacity 180ms ease-out';
        }}
        return;
      }}
      arm(ts1);
    }});
    requestAnimationFrame(arm);
  }}
  setInterval(update, 250);
  update();
}})();</script>"#,
        payload = serde_json::Value::Array(payload)
    )
}

fn supported_matrix(v: &str) -> bool {
    v == "none" || v.starts_with("matrix(") || v.starts_with("matrix3d(")
}

fn supported_inset_percent(v: &str) -> bool {
    if v == "none" {
        return true;
    }
    let Some(body) = v.strip_prefix("inset(").and_then(|s| s.strip_suffix(')')) else {
        return false;
    };
    let main = body.split(" round ").next().unwrap_or(body);
    !main.is_empty()
        && main
            .split_whitespace()
            .all(|part| part.ends_with('%') && part[..part.len() - 1].parse::<f64>().is_ok())
}

fn direct_style_supported(from: &serde_json::Value, to: &serde_json::Value) -> bool {
    let mut supported_change = false;
    let changed = |field: &str| from.get(field) != to.get(field);
    if changed("opacity") {
        let ok = from
            .get("opacity")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .is_some()
            && to
                .get("opacity")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<f64>().ok())
                .is_some();
        if !ok {
            return false;
        }
        supported_change = true;
    }
    if changed("borderRadius") {
        let ok = from
            .get("borderRadius")
            .and_then(|v| v.as_str())
            .and_then(|s| s.trim_end_matches("px").parse::<f64>().ok())
            .is_some()
            && to
                .get("borderRadius")
                .and_then(|v| v.as_str())
                .and_then(|s| s.trim_end_matches("px").parse::<f64>().ok())
                .is_some();
        if !ok {
            return false;
        }
        supported_change = true;
    }
    if changed("transform") {
        let ok = from
            .get("transform")
            .and_then(|v| v.as_str())
            .is_some_and(supported_matrix)
            && to
                .get("transform")
                .and_then(|v| v.as_str())
                .is_some_and(supported_matrix);
        if !ok {
            return false;
        }
        supported_change = true;
    }
    if changed("clipPath") {
        let ok = from
            .get("clipPath")
            .and_then(|v| v.as_str())
            .is_some_and(supported_inset_percent)
            && to
                .get("clipPath")
                .and_then(|v| v.as_str())
                .is_some_and(supported_inset_percent);
        if !ok {
            return false;
        }
        supported_change = true;
    }
    supported_change
}

fn direct_track_supported(effect: &serde_json::Value) -> bool {
    let Some(kind) = effect.get("kind").and_then(|v| v.as_str()) else {
        return false;
    };
    if effect.get("selector").and_then(|v| v.as_str()).is_none() {
        return false;
    }
    match kind {
        "scroll-pin" => {
            effect
                .get("pinnedSelector")
                .and_then(|v| v.as_str())
                .is_some()
                && effect.get("startY").and_then(|v| v.as_f64()).is_some()
                && effect.get("endY").and_then(|v| v.as_f64()).is_some()
                && effect
                    .get("desiredViewportTop")
                    .and_then(|v| v.as_f64())
                    .is_some()
                && effect.get("endY").and_then(|v| v.as_f64()).unwrap_or(0.0)
                    > effect.get("startY").and_then(|v| v.as_f64()).unwrap_or(0.0)
        }
        "scroll-coupled-visual-style" => {
            let Some(progress) = effect.get("progress") else {
                return false;
            };
            let Some(start) = progress.get("startY").and_then(|v| v.as_f64()) else {
                return false;
            };
            let Some(end) = progress.get("endY").and_then(|v| v.as_f64()) else {
                return false;
            };
            end > start
                && effect
                    .get("from")
                    .zip(effect.get("to"))
                    .is_some_and(|(from, to)| direct_style_supported(from, to))
        }
        "scroll-triggered-time-style" => {
            effect.get("triggerY").and_then(|v| v.as_f64()).is_some()
                && effect
                    .get("before")
                    .zip(effect.get("after"))
                    .is_some_and(|(before, after)| direct_style_supported(before, after))
        }
        _ => false,
    }
}

fn scroll_tracks_replay_script(effects: &[serde_json::Value]) -> String {
    let payload: Vec<serde_json::Value> = effects
        .iter()
        .filter(|e| {
            matches!(
                e.get("kind").and_then(|v| v.as_str()),
                Some("scroll-coupled-visual-style" | "scroll-pin" | "scroll-triggered-time-style")
            )
        })
        .cloned()
        .collect();
    if payload.is_empty() {
        return String::new();
    }
    format!(
        r#"<script id="hwatu-scroll-tracks-replay">(function () {{
  const cfg = {payload};
  const clamp = (v, a, b) => Math.max(a, Math.min(b, v));
  const lerp = (a, b, t) => a + (b - a) * t;
  const num = (v) => {{ const n = parseFloat(v); return Number.isFinite(n) ? n : null; }};
  const px = (v) => {{ const n = num(v); return n == null ? null : n + 'px'; }};
  const matrix = (v) => {{
    if (!v || v === 'none') return {{ sx: 1, sy: 1, tx: 0, ty: 0 }};
    let m = /^matrix\(([^)]+)\)$/.exec(v);
    if (m) {{ const p = m[1].split(',').map(parseFloat); return {{ sx: p[0] || 1, sy: p[3] || 1, tx: p[4] || 0, ty: p[5] || 0 }}; }}
    m = /^matrix3d\(([^)]+)\)$/.exec(v);
    if (m) {{ const p = m[1].split(',').map(parseFloat); return {{ sx: p[0] || 1, sy: p[5] || 1, tx: p[12] || 0, ty: p[13] || 0 }}; }}
    return null;
  }};
  const inset = (v) => {{
    const m = /^inset\(([^)]*)\)$/.exec(v || '');
    if (!m) return null;
    const [main, round] = m[1].split(/\s+round\s+/);
    const vals = main.trim().split(/\s+/).map(num);
    if (vals.some(x => x == null)) return null;
    while (vals.length < 4) vals.push(vals[vals.length === 1 ? 0 : vals.length - 2]);
    return {{ vals, round: round || '' }};
  }};
  const states = cfg.map((e) => {{
    const root = document.querySelector(e.selector || '');
    if (!root) return null;
    if (e.kind === 'scroll-pin') {{
      const child = document.querySelector(e.pinnedSelector || '') || root.firstElementChild || root;
      const baseTransform = child.style.transform || '';
      const computedTransform = getComputedStyle(child).transform;
      const baseline = baseTransform || (computedTransform && computedTransform !== 'none' ? computedTransform : '');
      return {{ e, root, child, baseline }};
    }}
    return {{ e, root }};
  }}).filter(Boolean);
  const writeStyle = (el, from, to, t) => {{
    if (!from || !to) return;
    const fo = num(from.opacity), toOp = num(to.opacity);
    if (fo != null && toOp != null && fo !== toOp) el.style.opacity = String(lerp(fo, toOp, t));
    const fr = px(from.borderRadius), tr = px(to.borderRadius);
    if (fr != null && tr != null && fr !== tr) el.style.borderRadius = lerp(parseFloat(fr), parseFloat(tr), t) + 'px';
    const fm = matrix(from.transform), tm = matrix(to.transform);
    if (fm && tm) el.style.transform = `translate(${{lerp(fm.tx, tm.tx, t)}}px, ${{lerp(fm.ty, tm.ty, t)}}px) scale(${{lerp(fm.sx, tm.sx, t)}}, ${{lerp(fm.sy, tm.sy, t)}})`;
    const fi = inset(from.clipPath), ti = inset(to.clipPath);
    if (fi && ti) el.style.clipPath = 'inset(' + fi.vals.map((v, i) => lerp(v, ti.vals[i], t).toFixed(3) + '%').join(' ') + (ti.round ? ' round ' + ti.round : '') + ')';
  }};
  const update = () => {{
    const y = scrollY || document.documentElement.scrollTop || 0;
    for (const s of states) {{
      const e = s.e;
      if (e.kind === 'scroll-pin') {{
        const start = +e.startY || 0, end = +e.endY || start;
        const active = y >= start && y <= end && end > start;
        if (!active) {{ s.child.style.transform = s.baseline; continue; }}
        const desired = Number.isFinite(+e.desiredViewportTop) ? +e.desiredViewportTop : s.child.getBoundingClientRect().top;
        // Restore the captured baseline before measuring so our previous
        // translate never feeds back into the next frame. The stable root
        // remains the activation anchor; this read only computes the final
        // correction needed for sticky/native layouts.
        s.child.style.transform = s.baseline;
        s.root.getBoundingClientRect();
        const delta = desired - s.child.getBoundingClientRect().top;
        s.child.style.transform = `translate3d(0, ${{delta}}px, 0)` + (s.baseline ? ' ' + s.baseline : '');
        s.child.style.willChange = 'transform';
      }} else if (e.kind === 'scroll-coupled-visual-style') {{
        const p = e.progress || {{}};
        const t = clamp((y - (+p.startY || 0)) / Math.max(1, (+p.endY || 0) - (+p.startY || 0)), 0, 1);
        writeStyle(s.root, e.from, e.to, t);
      }} else if (e.kind === 'scroll-triggered-time-style') {{
        const on = y >= (+e.triggerY || 0);
        s.root.dataset.hwatuTimeState = on ? 'in' : 'out';
        s.root.style.transition = 'transform 700ms cubic-bezier(.2,.8,.2,1), opacity 500ms ease, color 500ms ease, background-color 500ms ease, clip-path 700ms ease';
        writeStyle(s.root, on ? e.before : e.after, on ? e.after : e.before, 1);
      }}
    }}
  }};
  addEventListener('scroll', update, {{ passive: true }});
  document.addEventListener('scroll', update, {{ passive: true, capture: true }});
  if (typeof requestAnimationFrame === 'function') {{ const loop = () => {{ update(); requestAnimationFrame(loop); }}; requestAnimationFrame(loop); }}
  setInterval(update, 250);
  update();
}})();</script>"#,
        payload = serde_json::Value::Array(payload)
    )
}

fn scroll_effect_notice(effects: &[serde_json::Value]) -> Option<String> {
    if effects.is_empty() {
        return None;
    }
    let count = effects.len();
    let replayed = effects
        .iter()
        .filter(|e| e.get("replay").and_then(|v| v.as_str()) == Some("replayed"))
        .count();
    Some(format!(
        "\n<!-- hwatu: detected {count} scroll effect track(s): \
         {replayed} replayed by a generated scrollY->style runtime, {} report-only; \
         see scroll-effects.json for fits and entry/midpoint/exit samples. -->\n",
        count - replayed
    ))
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
            baseline_data: None,
            tolerance: opts.tolerance,
            heatmap: None,
            heatmap_data: false,
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

    #[test]
    fn scroll_effect_notice_names_replay_split() {
        let effects = vec![
            serde_json::json!({
                "kind": "scroll-coupled-text-style",
                "replay": "replayed",
            }),
            serde_json::json!({
                "kind": "scroll-coupled-text-style",
                "replay": "report-only",
            }),
        ];
        let notice = scroll_effect_notice(&effects).unwrap();
        assert!(
            notice.contains("detected 2 scroll effect track"),
            "{notice}"
        );
        assert!(notice.contains("1 replayed"), "{notice}");
        assert!(notice.contains("1 report-only"), "{notice}");
        assert!(notice.contains("scroll-effects.json"), "{notice}");
        assert!(scroll_effect_notice(&[]).is_none());
    }

    #[test]
    fn materialize_writes_scroll_effect_report() {
        let out =
            std::env::temp_dir().join(format!("hwatu-scroll-effect-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(out.join("assets")).unwrap();
        let cap = serde_json::json!({
            "base": "https://example.com/",
            "html": "<!doctype html><html><head></head><body><main data-hwatu-scroll-effect=\"0\">quote</main></body></html>",
            "sheets": [],
            "assets": [],
            "canvases": [],
            "scrolls": [],
            "pins": [],
            "scrollEffects": [{
                "kind": "scroll-coupled-text-style",
                "selector": "[data-hwatu-scroll-effect=\"0\"]",
                "changedTextNodes": 4,
                "samples": [
                    {"label": "entry", "scrollY": 100, "elements": []},
                    {"label": "midpoint", "scrollY": 200, "elements": []},
                    {"label": "exit", "scrollY": 300, "elements": []}
                ]
            }],
            "viewport": {"w": 800, "h": 600, "dpr": 1}
        });
        materialize(&cap, &out).unwrap();
        let report = std::fs::read_to_string(out.join("scroll-effects.json")).unwrap();
        assert!(
            report.contains("scroll-coupled text style mutation"),
            "{report}"
        );
        assert!(report.contains("midpoint"), "{report}");
        assert!(report.contains("report-only"), "{report}");
        let html = std::fs::read_to_string(out.join("index.html")).unwrap();
        assert!(html.contains("see scroll-effects.json"), "{html}");
        let _ = std::fs::remove_dir_all(&out);
    }

    /// Synthetic effect matching the runtime model with A=0.72, B=0.7
    /// on a 900px section at docTop=1500, innerHeight=720, 13 words.
    fn synthetic_effect(n_words: usize) -> serde_json::Value {
        let (doc_top, height, inner_height) = (1500.0, 900.0, 720.0);
        let (a, b) = (0.72, 0.7);
        let words: Vec<serde_json::Value> = (0..n_words)
            .map(|k| {
                // Flip threshold from the model: progress = t_k when
                // scrollY makes rect.top = docTop - y.
                let t = (k as f64 + 0.5) / n_words as f64;
                let flip_y = doc_top - inner_height * a + t * height * b;
                let states: Vec<serde_json::Value> = (0..=12)
                    .map(|s| {
                        let y = 800.0 + s as f64 * 100.0;
                        let lit = y >= flip_y;
                        serde_json::json!({
                            "y": y,
                            "color": if lit { "rgb(255, 255, 255)" } else { "rgba(255, 255, 255, 0.12)" },
                            "opacity": if lit { "1" } else { "0.42" },
                        })
                    })
                    .collect();
                serde_json::json!({ "i": k, "text": format!("w{k}"), "states": states })
            })
            .collect();
        serde_json::json!({
            "kind": "scroll-coupled-text-style",
            "selector": "[data-hwatu-scroll-effect=\"0\"]",
            "geometry": { "docTop": doc_top, "height": height, "innerHeight": inner_height },
            "words": words,
        })
    }

    #[test]
    fn scroll_effect_fit_recovers_model() {
        let fit = fit_scroll_effect(&synthetic_effect(13)).expect("fit");
        // Discrete 100px sampling quantizes each threshold; the
        // regression should still land near the true parameters.
        assert!((fit.a - 0.72).abs() < 0.12, "a={}", fit.a);
        assert!((fit.b - 0.7).abs() < 0.12, "b={}", fit.b);
        assert!(fit.r2 > SCROLL_FIT_MIN_R2, "r2={}", fit.r2);
        assert_eq!(fit.n_words, 13);
        assert_eq!(fit.muted_color, "rgba(255, 255, 255, 0.12)");
        assert_eq!(fit.lit_color, "rgb(255, 255, 255)");
        let script = scroll_replay_script(&[fit]);
        assert!(script.contains("hwatu-scroll-replay"), "{script}");
        assert!(script.contains("data-hwatu-scroll-word"), "{script}");
        // Hostile-environment drivers: scroll + rAF + interval.
        assert!(script.contains("addEventListener('scroll'"), "{script}");
        assert!(script.contains("requestAnimationFrame"), "{script}");
        assert!(script.contains("setInterval(update"), "{script}");
    }

    #[test]
    fn scroll_effect_fit_gates_on_quality() {
        // No words at all: report-only.
        assert!(fit_scroll_effect(&serde_json::json!({
            "selector": "[data-hwatu-scroll-effect=\"0\"]",
            "geometry": { "docTop": 100, "height": 900, "innerHeight": 720 },
        }))
        .is_none());
        // Random (non-monotone-in-index) flips: r² below the gate.
        let mut effect = synthetic_effect(8);
        let words = effect["words"].as_array_mut().unwrap();
        words.reverse(); // reverses flip order vs index -> negative slope
        for (k, w) in words.iter_mut().enumerate() {
            w["i"] = serde_json::json!(k);
        }
        assert!(fit_scroll_effect(&effect).is_none());
    }

    #[test]
    fn direct_tracks_gate_unsupported_shapes() {
        let visual = serde_json::json!({
            "kind": "scroll-coupled-visual-style",
            "selector": "#visual",
            "progress": { "startY": 10, "endY": 100 },
            "from": { "opacity": "0", "transform": "matrix(1, 0, 0, 1, 0, 0)", "clipPath": "inset(0% 100% 0% 0% round 10px)" },
            "to": { "opacity": "1", "transform": "matrix(2, 0, 0, 2, 0, 0)", "clipPath": "inset(0% 0% 0% 0% round 10px)" }
        });
        assert!(direct_track_supported(&visual));
        let unsupported = serde_json::json!({
            "kind": "scroll-coupled-visual-style",
            "selector": "#visual",
            "progress": { "startY": 10, "endY": 100 },
            "from": { "clipPath": "circle(10px at 50% 50%)" },
            "to": { "clipPath": "circle(50px at 50% 50%)" }
        });
        assert!(!direct_track_supported(&unsupported));
    }

    #[test]
    fn scroll_tracks_runtime_composes_pin_baseline_transform() {
        let script = scroll_tracks_replay_script(&[serde_json::json!({
            "kind": "scroll-pin",
            "selector": "#pin-root",
            "pinnedSelector": "#pinned",
            "startY": 100,
            "endY": 900,
            "desiredViewportTop": 120,
            "replay": "replayed"
        })]);
        assert!(script.contains("hwatu-scroll-tracks-replay"), "{script}");
        assert!(script.contains("baseTransform"), "{script}");
        assert!(script.contains("computedTransform"), "{script}");
        assert!(
            script.contains("s.child.style.transform = s.baseline"),
            "{script}"
        );
        assert!(
            script.contains("+ (s.baseline ? ' ' + s.baseline"),
            "{script}"
        );
        assert!(script.contains("translate3d(0,"), "{script}");
    }

    #[test]
    fn linear_regress_recovers_line() {
        let pts: Vec<(f64, f64)> = (0..10).map(|i| (i as f64, 3.0 + 2.0 * i as f64)).collect();
        let (slope, intercept, r2) = linear_regress(&pts).unwrap();
        assert!((slope - 2.0).abs() < 1e-9);
        assert!((intercept - 3.0).abs() < 1e-9);
        assert!((r2 - 1.0).abs() < 1e-9);
        assert!(linear_regress(&[(1.0, 2.0)]).is_none());
        assert!(linear_regress(&[(1.0, 2.0), (1.0, 3.0)]).is_none());
    }

    #[test]
    fn unreplicated_motion_maps_fitted_tracks() {
        let motion = serde_json::json!({
            "observed": [
                { "model": "linear", "target": "ul.marquee", "velocity_px_s": -30.0,
                  "period_s": 28.0, "fit_r2": 0.99 },
                { "model": "periodic", "target": "div.bob", "period_s": 1.5, "fit_r2": 0.8 },
                { "model": "bezier", "target": "aside.slide", "duration_ms": 300.0,
                  "fit_r2": 0.97 },
            ],
        });
        let entries = unreplicated_motion_entries(&motion);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0]["kind"], "loop");
        assert_eq!(entries[0]["selector"], "ul.marquee");
        assert_eq!(entries[0]["period_ms"], 28_000.0);
        assert_eq!(entries[0]["r2"], 0.99);
        assert_eq!(entries[1]["kind"], "oscillation");
        assert_eq!(entries[1]["period_ms"], 1_500.0);
        assert_eq!(entries[2]["kind"], "one-shot");
        assert_eq!(entries[2]["period_ms"], 300.0);
        assert!(unreplicated_motion_entries(&serde_json::json!({})).is_empty());
    }
}
