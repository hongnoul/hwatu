// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! Trusted native input support (issue #23).
//!
//! The page cannot mint `isTrusted: true` events and neither can
//! WebKitGTK's public API, so trusted input must enter through the
//! compositor like a real mouse. Two hard problems fall out of that:
//! (1) where is the element in *global* coordinates, and (2) which
//! surface will the compositor route the events to.
//!
//! Both are attacked with the same two moves:
//!
//! * **Fullscreen for the duration of the injection.** A fullscreen
//!   surface coincides with its monitor, so surface-local coordinates
//!   are (approximately) output-local coordinates and no
//!   compositor-specific window-position query is needed. niri, for
//!   one, does not expose window positions over IPC
//!   (`tile_pos_in_workspace_view` may be null), and its view-scroll
//!   animations mean any static origin guess can land input in a
//!   neighboring column mid-animation.
//!
//! * **Closed-loop calibration before the button press.** The injected
//!   all-frames resolver records trusted `mousemove` positions (child
//!   frames forward theirs to the top frame translated into
//!   top-viewport coordinates). The daemon probes with virtual-pointer
//!   motion, reads back where the page *actually* saw the pointer,
//!   corrects by the observed delta, and only clicks once intended and
//!   observed positions agree twice in a row. This absorbs remaining
//!   unknowns: fullscreen animations, window chrome, HiDPI scale, page
//!   zoom.
//!
//! The page-side selector half is an all-frames resolver: the top
//! frame asks same-origin or cross-origin child frames for a
//! selector's viewport rect via `postMessage`; child frames answer
//! from injected user-script code, not from page JS access.

use crate::window::BrowserWindow;
use gtk::glib;
use gtk::prelude::*;
use hwatu_ipc::OpenMode;
use serde::Deserialize;
use serde_json::Value;
use std::cell::Cell;
use std::process::Command;
use std::rc::Rc;
use std::time::Duration;
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_output, wl_pointer, wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};
use webkit6::prelude::*;

pub const RESOLVER_JS: &str = r#"(() => {
  if (window.__hwatuTrustedResolveInstalled) return;
  window.__hwatuTrustedResolveInstalled = true;

  // ---- trusted pointer observation (calibration channel) ----
  // Every frame records trusted mousemoves. Child frames forward theirs
  // to the parent, which translates into its own viewport coordinates
  // and re-forwards, so the top frame stores a unified observation.
  const isTop = (() => { try { return window === window.top; } catch (_) { return false; } })();
  window.__hwatuMoveSeq = 0;
  const recordMove = (x, y) => {
    if (isTop) {
      window.__hwatuLastTrustedMove = { x, y, seq: ++window.__hwatuMoveSeq };
    } else {
      try { parent.postMessage({ __hwatuTrustedMove: true, x, y }, '*'); } catch (_) {}
    }
  };
  document.addEventListener('mousemove', (event) => {
    if (event.isTrusted) recordMove(event.clientX, event.clientY);
  }, { capture: true, passive: true });
  window.addEventListener('message', (event) => {
    const msg = event.data;
    if (!msg || msg.__hwatuTrustedMove !== true) return;
    const frame = [...document.querySelectorAll('iframe')]
      .find(f => f.contentWindow === event.source);
    if (!frame) return;
    const r = frame.getBoundingClientRect();
    recordMove(msg.x + r.left, msg.y + r.top);
  }, true);

  // ---- selector resolution across frames ----
  const ownText = (el) => ((el.textContent || '') + ' ' + (el.value || '')).trim();
  const select = (selector, nth, contains, refIdx) => {
    let el;
    let total = 0;
    let matches = 0;
    if (refIdx !== null && refIdx !== undefined) {
      const refs = window.__hwatu_refs;
      if (!refs) return null;
      el = refs[refIdx];
      total = refs.length;
      matches = refs.length;
      if (!el || !el.isConnected) return null;
    } else {
      let els;
      try { els = [...document.querySelectorAll(selector)]; }
      catch (_) { return null; }
      total = els.length;
      if (contains !== null && contains !== undefined)
        els = els.filter(e => ownText(e).includes(contains));
      matches = els.length;
      el = els[nth || 0];
      if (!el) return null;
    }
    try { el.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' }); } catch (_) {}
    const r = el.getBoundingClientRect();
    if (!Number.isFinite(r.left) || !Number.isFinite(r.top) || r.width <= 0 || r.height <= 0)
      return null;
    return {
      x: r.left + r.width / 2,
      y: r.top + r.height / 2,
      width: r.width,
      height: r.height,
      viewportWidth: window.innerWidth,
      viewportHeight: window.innerHeight,
      frameUrl: location.href,
      matched: {
        matches,
        total,
        tag: el.tagName ? el.tagName.toLowerCase() : '',
        text: ownText(el).slice(0, 120)
      }
    };
  };

  window.addEventListener('message', (event) => {
    const req = event.data;
    if (!req || req.__hwatuTrustedResolve !== true) return;
    const found = select(req.selector, req.nth, req.contains, req.refIdx);
    if (!found) return;
    event.source && event.source.postMessage({
      __hwatuTrustedResolveReply: true,
      token: req.token,
      found
    }, '*');
  }, true);

  window.__hwatuTrustedResolve = (selector, nth, contains, refIdx) => new Promise((resolve, reject) => {
    const local = select(selector, nth, contains, refIdx);
    if (local) {
      local.frame = 'top';
      resolve(local);
      return;
    }

    const frames = [...document.querySelectorAll('iframe')]
      .filter(frame => frame && frame.contentWindow);
    if (!frames.length) {
      reject(new Error(`no match for trusted selector ${JSON.stringify(selector)}`));
      return;
    }

    const token = `hwatu:${Date.now()}:${Math.random()}`;
    const done = (value, error) => {
      clearTimeout(timer);
      window.removeEventListener('message', onReply, true);
      error ? reject(error) : resolve(value);
    };
    const onReply = (event) => {
      const msg = event.data;
      if (!msg || msg.__hwatuTrustedResolveReply !== true || msg.token !== token || !msg.found)
        return;
      const frame = frames.find(f => f.contentWindow === event.source);
      if (!frame) return;
      const fr = frame.getBoundingClientRect();
      const found = msg.found;
      found.x += fr.left;
      found.y += fr.top;
      // x/y are now top-viewport coordinates; report the top viewport
      // size too so range checks stay coherent.
      found.viewportWidth = window.innerWidth;
      found.viewportHeight = window.innerHeight;
      found.frame = 'iframe';
      found.iframe = { id: frame.id || '', src: frame.src || '', x: fr.left, y: fr.top, width: fr.width, height: fr.height };
      done(found, null);
    };
    const timer = setTimeout(() => done(null, new Error(`no match for trusted selector ${JSON.stringify(selector)} in top document or child frames`)), 1000);
    window.addEventListener('message', onReply, true);
    const req = { __hwatuTrustedResolve: true, token, selector, nth, contains, refIdx };
    for (const frame of frames) frame.contentWindow.postMessage(req, '*');
  });
})();"#;

pub fn wire_view(view: &webkit6::WebView) {
    let Some(ucm) = view.user_content_manager() else {
        return;
    };
    let script = webkit6::UserScript::new(
        RESOLVER_JS,
        webkit6::UserContentInjectedFrames::AllFrames,
        webkit6::UserScriptInjectionTime::Start,
        &[],
        &[],
    );
    ucm.add_script(&script);
}

#[derive(Debug, Clone, Deserialize)]
pub struct TargetPoint {
    pub x: f64,
    pub y: f64,
    #[serde(rename = "viewportWidth")]
    pub viewport_width: f64,
    #[serde(rename = "viewportHeight")]
    pub viewport_height: f64,
    #[serde(default)]
    pub matched: Value,
    #[serde(default)]
    pub frame: String,
    #[serde(default)]
    pub iframe: Value,
}

#[derive(Debug, Clone)]
pub struct InputReport {
    pub backend: &'static str,
    /// Final compositor-global coordinates the button was pressed at.
    pub x: i64,
    pub y: i64,
    /// Extents the absolute motion was expressed against (monitor
    /// logical size).
    pub extent_width: u32,
    pub extent_height: u32,
    /// How many motion probes the calibration loop needed.
    pub probes: u32,
}

/// Window-state guard for a trusted-input run: promotes + fullscreens
/// the window on `prepare`, restores the previous state and compositor
/// focus on `finish` (idempotent, shared across the racing callbacks via
/// `Clone`).
#[derive(Clone)]
pub struct TrustedSession {
    win: Rc<BrowserWindow>,
    was_fullscreen: bool,
    promoted: bool,
    prior_niri_focus: Option<u64>,
    finished: Rc<Cell<bool>>,
}

pub fn prepare(win: &Rc<BrowserWindow>) -> TrustedSession {
    let was_fullscreen = win.window.is_fullscreen();
    let promoted = win.mode() != OpenMode::Normal;
    // Native trusted input must temporarily focus a mapped compositor
    // surface. Remember what the user was actually using before doing so;
    // merely hiding the promoted headless window afterwards lets the WM
    // choose an arbitrary successor and visibly strands keyboard focus.
    let prior_niri_focus = focused_niri_window();
    // Best-effort compositor-side focus first: on niri this also switches
    // to the window's workspace, which fullscreen alone does not do.
    if let Ok(id) = find_niri_window(win) {
        let _ = focus_niri_window(id);
    }
    win.present();
    if !was_fullscreen {
        win.window.fullscreen();
    }
    TrustedSession {
        win: win.clone(),
        was_fullscreen,
        promoted,
        prior_niri_focus,
        finished: Rc::new(Cell::new(false)),
    }
}

impl TrustedSession {
    pub fn finish(&self) {
        if self.finished.replace(true) {
            return;
        }
        if !self.was_fullscreen {
            self.win.window.unfullscreen();
        }
        if self.promoted {
            self.win.unfocus();
        }
        if let Some(id) = self.prior_niri_focus {
            let _ = focus_niri_window(id);
        }
    }
}

/// Wait (polling, non-blocking for the main loop) until the window is
/// actually fullscreen-sized, then give the page one relayout beat.
pub fn when_fullscreen(
    win: &Rc<BrowserWindow>,
    wait_ms: u64,
    cb: impl FnOnce(Result<(), String>) + 'static,
) {
    let win = win.clone();
    let deadline = std::time::Instant::now() + Duration::from_millis(wait_ms);
    let cb = Cell::new(Some(cb));
    glib::timeout_add_local(Duration::from_millis(30), move || {
        let ready = win.window.is_fullscreen() && {
            match monitor_logical_size(&win) {
                Some((mw, _)) => win.window.width() >= mw - 2,
                None => true,
            }
        };
        if ready {
            if let Some(cb) = cb.take() {
                // One extra beat for the page relayout after resize.
                glib::timeout_add_local_once(Duration::from_millis(120), move || cb(Ok(())));
            }
            return glib::ControlFlow::Break;
        }
        if std::time::Instant::now() >= deadline {
            if let Some(cb) = cb.take() {
                cb(Err(format!(
                    "window did not reach fullscreen within {wait_ms} ms"
                )));
            }
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
}

fn monitor_logical_size(win: &Rc<BrowserWindow>) -> Option<(i32, i32)> {
    let surface = win.window.surface()?;
    let display = gtk::prelude::WidgetExt::display(&win.window);
    let monitor = display.monitor_at_surface(&surface)?;
    let geo = monitor.geometry();
    Some((geo.width(), geo.height()))
}

pub async fn inject_click(
    win: &Rc<BrowserWindow>,
    view: &webkit6::WebView,
    target: &TargetPoint,
) -> Result<InputReport, String> {
    let mut pointer = calibrate(win, view, target).await?;
    pointer.click()?;
    glib::timeout_future(Duration::from_millis(30)).await;
    Ok(pointer.report)
}

pub async fn inject_type(
    win: &Rc<BrowserWindow>,
    view: &webkit6::WebView,
    target: &TargetPoint,
    text: &str,
    clear: bool,
    enter: bool,
) -> Result<InputReport, String> {
    // The click both proves delivery (calibration) and moves keyboard
    // focus to the target's window + element.
    let report = inject_click(win, view, target).await?;
    glib::timeout_future(Duration::from_millis(60)).await;
    if clear {
        run_wtype(&["-M", "ctrl", "a", "-m", "ctrl", "-k", "BackSpace"])?;
        glib::timeout_future(Duration::from_millis(30)).await;
    }
    if !text.is_empty() {
        run_wtype(&["--", text])?;
    }
    if enter {
        glib::timeout_future(Duration::from_millis(30)).await;
        run_wtype(&["-k", "Return"])?;
    }
    // wtype returns once the events are queued, not once WebKit has
    // delivered them to the page. Settle proportionally to the text
    // length so callers that read page state right after us see the
    // final value.
    let settle = 120 + 8 * text.chars().count() as u64;
    glib::timeout_future(Duration::from_millis(settle.min(1200))).await;
    Ok(report)
}

pub async fn inject_paste(
    win: &Rc<BrowserWindow>,
    view: &webkit6::WebView,
    target: &TargetPoint,
) -> Result<InputReport, String> {
    // The click both proves delivery (calibration) and moves keyboard
    // focus to the target's window + element before wtype sends Ctrl+V.
    let report = inject_click(win, view, target).await?;
    glib::timeout_future(Duration::from_millis(60)).await;
    run_wtype(&["-M", "ctrl", "v", "-m", "ctrl"])?;
    glib::timeout_future(Duration::from_millis(180)).await;
    Ok(report)
}

struct WlState;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for WlState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for WlState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_seat::WlSeat,
        _event: wl_seat::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_output::WlOutput, ()> for WlState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_output::WlOutput,
        _event: wl_output::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrVirtualPointerManagerV1, ()> for WlState {
    fn event(
        _state: &mut Self,
        _proxy: &ZwlrVirtualPointerManagerV1,
        _event: wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_manager_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrVirtualPointerV1, ()> for WlState {
    fn event(
        _state: &mut Self,
        _proxy: &ZwlrVirtualPointerV1,
        _event: wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

/// An open virtual-pointer session, positioned over the calibrated
/// target and ready to press the button.
struct CalibratedPointer {
    conn: Connection,
    queue: EventQueue<WlState>,
    pointer: ZwlrVirtualPointerV1,
    report: InputReport,
}

impl CalibratedPointer {
    fn motion(&mut self, x: f64, y: f64) -> Result<(), String> {
        let cx = x
            .round()
            .clamp(0.0, self.report.extent_width.saturating_sub(1) as f64) as u32;
        let cy = y
            .round()
            .clamp(0.0, self.report.extent_height.saturating_sub(1) as f64) as u32;
        self.pointer.motion_absolute(
            monotonic_millis(),
            cx,
            cy,
            self.report.extent_width,
            self.report.extent_height,
        );
        self.pointer.frame();
        self.flush()
    }

    fn click(&mut self) -> Result<(), String> {
        let time = monotonic_millis();
        // BTN_LEFT = 0x110 (input-event-codes.h).
        self.pointer
            .button(time, 0x110, wl_pointer::ButtonState::Pressed);
        self.pointer.frame();
        self.pointer.button(
            time.wrapping_add(4),
            0x110,
            wl_pointer::ButtonState::Released,
        );
        self.pointer.frame();
        self.flush()
    }

    fn flush(&mut self) -> Result<(), String> {
        self.conn
            .flush()
            .map_err(|e| format!("failed to flush trusted pointer events: {e}"))?;
        let mut state = WlState;
        let _ = self.queue.roundtrip(&mut state);
        Ok(())
    }
}

fn open_pointer(extent_width: u32, extent_height: u32) -> Result<CalibratedPointer, String> {
    let conn = Connection::connect_to_env()
        .map_err(|e| format!("failed to connect to Wayland display for trusted pointer: {e}"))?;
    let (globals, mut queue) = registry_queue_init::<WlState>(&conn)
        .map_err(|e| format!("failed to read Wayland globals for trusted pointer: {e}"))?;
    let qh = queue.handle();
    let manager: ZwlrVirtualPointerManagerV1 = globals.bind(&qh, 1..=2, ()).map_err(|e| {
        format!("compositor does not advertise zwlr_virtual_pointer_manager_v1: {e}")
    })?;
    let seat: Option<wl_seat::WlSeat> = globals.bind(&qh, 1..=9, ()).ok();
    let output: Option<wl_output::WlOutput> = globals.bind(&qh, 1..=4, ()).ok();
    let pointer = manager.create_virtual_pointer_with_output::<_, WlState>(
        seat.as_ref(),
        output.as_ref(),
        &qh,
        (),
    );
    let mut state = WlState;
    conn.flush()
        .map_err(|e| format!("failed to flush trusted pointer creation: {e}"))?;
    let _ = queue.roundtrip(&mut state);
    Ok(CalibratedPointer {
        conn,
        queue,
        pointer,
        report: InputReport {
            backend: "fullscreen+wlr-virtual-pointer(calibrated)+wtype",
            x: 0,
            y: 0,
            extent_width,
            extent_height,
            probes: 0,
        },
    })
}

/// Read the top frame's last unified trusted-mousemove observation.
async fn read_last_move(view: &webkit6::WebView) -> Result<Option<(f64, f64, u64)>, String> {
    let value = view
        .call_async_javascript_function_future(
            "return window.__hwatuLastTrustedMove || null;",
            None,
            None,
            None,
        )
        .await
        .map_err(|e| format!("trusted pointer calibration readback failed: {e}"))?;
    if value.is_null() || value.is_undefined() {
        return Ok(None);
    }
    let json: Value = value
        .to_json(0)
        .and_then(|s| serde_json::from_str(s.as_str()).ok())
        .ok_or("trusted pointer calibration readback returned non-JSON")?;
    let x = json.get("x").and_then(Value::as_f64);
    let y = json.get("y").and_then(Value::as_f64);
    let seq = json.get("seq").and_then(Value::as_u64);
    match (x, y, seq) {
        (Some(x), Some(y), Some(seq)) => Ok(Some((x, y, seq))),
        _ => Ok(None),
    }
}

/// Iterate virtual-pointer probes until the page's observed pointer
/// position matches the intended target, then return the open pointer
/// session parked exactly over the target. The window is expected to
/// already be fullscreen (see [`prepare`] / [`when_fullscreen`]).
async fn calibrate(
    win: &Rc<BrowserWindow>,
    view: &webkit6::WebView,
    target: &TargetPoint,
) -> Result<CalibratedPointer, String> {
    let (mon_w, mon_h) = monitor_logical_size(win)
        .ok_or("could not determine the window's monitor geometry for trusted input")?;

    if target.viewport_width > 0.0
        && target.viewport_height > 0.0
        && (target.x < 0.0
            || target.y < 0.0
            || target.x > target.viewport_width
            || target.y > target.viewport_height)
    {
        return Err(format!(
            "trusted target center ({:.0},{:.0}) is outside the {}x{} viewport; \
             scroll it into view first",
            target.x, target.y, target.viewport_width, target.viewport_height
        ));
    }

    // Fullscreen surface == monitor, so start from origin (0,0) and let
    // the probe loop absorb chrome offsets / animations / zoom.
    let mut origin_x = 0.0f64;
    let mut origin_y = 0.0f64;

    let mut pointer = open_pointer(mon_w.max(1) as u32, mon_h.max(1) as u32)?;

    let mut last_seq = read_last_move(view).await?.map(|(_, _, s)| s).unwrap_or(0);
    let mut consecutive_hits = 0u32;
    let mut last_error = String::from("no trusted mousemove was observed by the page");

    for attempt in 0..14u32 {
        let gx = (origin_x + target.x).round();
        let gy = (origin_y + target.y).round();
        // Approach from a nearby point so the compositor always has a
        // fresh motion to deliver, even when the pointer already rests
        // on the target position.
        let wiggle = if attempt % 2 == 0 { 2.0 } else { 3.0 };
        pointer.motion(gx, gy - wiggle)?;
        glib::timeout_future(Duration::from_millis(15)).await;
        pointer.motion(gx, gy)?;
        glib::timeout_future(Duration::from_millis(70)).await;

        pointer.report.probes = attempt + 1;
        let observed = read_last_move(view).await?;
        let Some((ox, oy, seq)) = observed else {
            // Nothing seen yet: surface may still be animating into
            // place, or another surface is under the pointer.
            glib::timeout_future(Duration::from_millis(120)).await;
            continue;
        };
        if seq == last_seq {
            last_error = format!(
                "page stopped observing pointer motion (last saw viewport {ox:.0},{oy:.0}); \
                 the window may be occluded or on another workspace"
            );
            // A stable stall usually means the window lost its
            // fullscreen/focused state to a racing configure (e.g. a
            // previous trusted session's restore was still in flight
            // when this one started). Re-assert it and retry.
            if attempt % 3 == 2 {
                if let Ok(id) = find_niri_window(win) {
                    let _ = Command::new("niri")
                        .args(["msg", "action", "focus-window", "--id", &id.to_string()])
                        .status();
                }
                win.present();
                win.window.unfullscreen();
                glib::timeout_future(Duration::from_millis(80)).await;
                win.window.fullscreen();
                glib::timeout_future(Duration::from_millis(250)).await;
            } else {
                glib::timeout_future(Duration::from_millis(120)).await;
            }
            continue;
        }
        last_seq = seq;

        let dvx = target.x - ox;
        let dvy = target.y - oy;
        if dvx.abs() <= 1.5 && dvy.abs() <= 1.5 {
            consecutive_hits += 1;
            if consecutive_hits >= 2 {
                pointer.report.x = gx as i64;
                pointer.report.y = gy as i64;
                return Ok(pointer);
            }
            // Confirm stability with one more probe (the surface may
            // still be moving underneath us).
            glib::timeout_future(Duration::from_millis(60)).await;
            continue;
        }
        consecutive_hits = 0;
        last_error = format!(
            "calibration delta {dvx:.1},{dvy:.1} viewport px after probe {}",
            attempt + 1
        );
        origin_x += dvx;
        origin_y += dvy;
    }

    Err(format!(
        "trusted pointer calibration did not converge: {last_error}"
    ))
}

fn monotonic_millis() -> u32 {
    std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
        .map(|secs| (secs * 1000.0) as u32)
        .unwrap_or(0)
}

fn run_wtype(args: &[&str]) -> Result<(), String> {
    let status = Command::new("wtype")
        .args(args)
        .status()
        .map_err(|e| format!("failed to run wtype for trusted text input: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("wtype {} exited with {status}", args.join(" ")))
    }
}

fn focus_niri_window(id: u64) -> Result<(), String> {
    let status = Command::new("niri")
        .args(["msg", "action", "focus-window", "--id", &id.to_string()])
        .status()
        .map_err(|e| format!("failed to focus niri window {id}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("niri failed to focus window {id}: {status}"))
    }
}

fn focused_niri_window_id(windows: &Value) -> Option<u64> {
    let focused = windows
        .as_array()?
        .iter()
        .find(|window| window.get("is_focused").and_then(Value::as_bool) == Some(true))?;
    focused.get("id").and_then(Value::as_u64)
}

fn focused_niri_window() -> Option<u64> {
    let out = Command::new("niri")
        .args(["msg", "--json", "windows"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let windows: Value = serde_json::from_slice(&out.stdout).ok()?;
    focused_niri_window_id(&windows)
}

/// Best-effort niri window lookup (pid + title). Used only to ask niri
/// to focus the window (which also switches to its workspace); all
/// coordinate work is calibration-based and compositor-agnostic.
fn find_niri_window(win: &Rc<BrowserWindow>) -> Result<u64, String> {
    let pid = std::process::id() as u64;
    let title = win
        .window
        .title()
        .map(|s| s.to_string())
        .unwrap_or_default();
    let out = Command::new("niri")
        .args(["msg", "--json", "windows"])
        .output()
        .map_err(|e| format!("failed to run niri msg windows: {e}"))?;
    if !out.status.success() {
        return Err("niri msg windows failed".into());
    }
    let windows: Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("niri msg windows returned invalid JSON: {e}"))?;
    let arr = windows
        .as_array()
        .ok_or("niri windows response was not an array")?;
    let mut candidates: Vec<&Value> = arr
        .iter()
        .filter(|w| w.get("pid").and_then(Value::as_u64) == Some(pid))
        .collect();
    if !title.is_empty() {
        let exact: Vec<&Value> = candidates
            .iter()
            .copied()
            .filter(|w| w.get("title").and_then(Value::as_str) == Some(title.as_str()))
            .collect();
        if !exact.is_empty() {
            candidates = exact;
        }
    }
    if candidates.len() == 1 {
        return candidates[0]
            .get("id")
            .and_then(Value::as_u64)
            .ok_or_else(|| "matching niri window had no id".to_string());
    }
    Err(format!(
        "could not uniquely match hwatu window in niri IPC (candidates={})",
        candidates.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::{focused_niri_window_id, RESOLVER_JS};
    use serde_json::json;

    #[test]
    fn resolver_is_all_frames_shape() {
        assert!(RESOLVER_JS.contains("__hwatuTrustedResolve"));
        assert!(RESOLVER_JS.contains("postMessage"));
        assert!(RESOLVER_JS.contains("iframe"));
    }

    #[test]
    fn resolver_records_trusted_moves_for_calibration() {
        assert!(RESOLVER_JS.contains("__hwatuLastTrustedMove"));
        assert!(RESOLVER_JS.contains("__hwatuTrustedMove"));
        assert!(RESOLVER_JS.contains("mousemove"));
    }

    #[test]
    fn focused_niri_window_id_selects_only_focused_window() {
        let windows = json!([
            {"id": 10, "is_focused": false},
            {"id": 11, "is_focused": true},
            {"id": 12, "is_focused": false}
        ]);
        assert_eq!(focused_niri_window_id(&windows), Some(11));
        assert_eq!(focused_niri_window_id(&json!([])), None);
        assert_eq!(focused_niri_window_id(&json!({"id": 11})), None);
    }
}
