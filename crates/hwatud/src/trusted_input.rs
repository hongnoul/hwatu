// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! Trusted native input support.
//!
//! The page-side half is a tiny all-frames resolver. The top frame can ask
//! same-origin or cross-origin child frames for a selector's viewport rect via
//! `postMessage`; child frames answer from injected user-script code, not from
//! page JS access. The daemon then converts the returned WebView-local CSS
//! coordinates into compositor-global coordinates and injects real input.

use crate::window::BrowserWindow;
use gtk::prelude::*;
use serde::Deserialize;
use serde_json::Value;
use std::process::Command;
use std::rc::Rc;
use std::time::Duration;
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_output, wl_pointer, wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};
use webkit6::prelude::*;

pub const RESOLVER_JS: &str = r#"(() => {
  if (window.__hwatuTrustedResolveInstalled) return;
  window.__hwatuTrustedResolveInstalled = true;

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

pub fn inject_click(win: &Rc<BrowserWindow>, target: &TargetPoint) -> Result<InputReport, String> {
    let report = focus_and_map(win, target)?;
    wayland_pointer_click(&report)?;
    std::thread::sleep(Duration::from_millis(20));
    Ok(report)
}

pub fn inject_type(
    win: &Rc<BrowserWindow>,
    target: &TargetPoint,
    text: &str,
    clear: bool,
    enter: bool,
) -> Result<InputReport, String> {
    let report = inject_click(win, target)?;
    std::thread::sleep(Duration::from_millis(30));
    if clear {
        // Linux evdev keycodes: leftctrl=29, a=30, backspace=14.
        run_ydotool(&["key", "29:1", "30:1", "30:0", "29:0", "14:1", "14:0"])?;
    }
    if !text.is_empty() {
        run_wtype(text)?;
    }
    if enter {
        run_wtype("\n")?;
    }
    Ok(report)
}

#[derive(Debug, Clone)]
pub struct InputReport {
    pub backend: &'static str,
    pub niri_window_id: u64,
    pub x: i64,
    pub y: i64,
    pub window_origin_x: f64,
    pub window_origin_y: f64,
    pub window_width: f64,
    pub window_height: f64,
    pub extent_width: u32,
    pub extent_height: u32,
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

fn wayland_pointer_click(report: &InputReport) -> Result<(), String> {
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

    let x = report.x.clamp(0, report.extent_width as i64) as u32;
    let y = report.y.clamp(0, report.extent_height as i64) as u32;
    let time = monotonic_millis();
    pointer.motion_absolute(time, x, y, report.extent_width, report.extent_height);
    pointer.frame();
    pointer.button(
        time.wrapping_add(1),
        0x110,
        wl_pointer::ButtonState::Pressed,
    );
    pointer.frame();
    pointer.button(
        time.wrapping_add(2),
        0x110,
        wl_pointer::ButtonState::Released,
    );
    pointer.frame();
    conn.flush()
        .map_err(|e| format!("failed to flush trusted pointer events: {e}"))?;
    let _ = queue.roundtrip(&mut state);
    Ok(())
}

fn monotonic_millis() -> u32 {
    std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
        .map(|secs| (secs * 1000.0) as u32)
        .unwrap_or(0)
}

fn run_wtype(text: &str) -> Result<(), String> {
    let status = Command::new("wtype")
        .arg(text)
        .status()
        .map_err(|e| format!("failed to run wtype for trusted text input: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("wtype exited with {status}"))
    }
}

fn run_ydotool(args: &[&str]) -> Result<(), String> {
    let status = Command::new("ydotool")
        .args(args)
        .status()
        .map_err(|e| format!("failed to run ydotool: {e}. Install/enable ydotoold or set up a Wayland virtual input backend"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("ydotool {} exited with {status}", args.join(" ")))
    }
}

fn focus_and_map(win: &Rc<BrowserWindow>, target: &TargetPoint) -> Result<InputReport, String> {
    let niri_id = find_niri_window(win)?;
    let status = Command::new("niri")
        .args([
            "msg",
            "action",
            "focus-window",
            "--id",
            &niri_id.to_string(),
        ])
        .status()
        .map_err(|e| format!("failed to focus niri window {niri_id}: {e}"))?;
    if !status.success() {
        return Err(format!("niri failed to focus window {niri_id}: {status}"));
    }
    std::thread::sleep(Duration::from_millis(80));

    let focused = niri_window_by_id(niri_id)?;
    let output_name = workspace_output(
        focused
            .get("workspace_id")
            .and_then(Value::as_u64)
            .ok_or("niri focused-window did not include workspace_id")?,
    )?;
    let output = output_geometry(&output_name)?;
    let layout = focused
        .get("layout")
        .ok_or("niri focused-window did not include layout")?;
    let (win_w, win_h) = pair_f64(
        layout
            .get("window_size")
            .ok_or("niri focused-window layout did not include window_size")?,
    )?;
    let (off_x, off_y) = pair_f64(
        layout
            .get("window_offset_in_tile")
            .unwrap_or(&Value::Array(vec![Value::from(0), Value::from(0)])),
    )?;

    // niri does not expose absolute window coordinates in `windows` today.
    // After `focus-window`, the focused column is centered horizontally in the
    // output view. Vertically, the window consumes the output's usable area
    // below layer-shell bars plus the configured outer gap. Infer that top edge
    // from output height and niri's reported window height. This is intentionally
    // constrained to niri and returns an actionable error when the fields differ.
    let horizontal_slack = (output.width - win_w).max(0.0);
    let vertical_slack = (output.height - win_h).max(0.0);
    let side_gap = if horizontal_slack <= 64.0 {
        horizontal_slack / 2.0
    } else {
        8.0
    };
    let origin_x = output.x + horizontal_slack / 2.0 + off_x;
    let origin_y = output.y + (vertical_slack - side_gap).max(0.0) + off_y;

    let scale_x = if target.viewport_width > 0.0 {
        win_w / target.viewport_width
    } else {
        1.0
    };
    let scale_y = if target.viewport_height > 0.0 {
        win_h / target.viewport_height
    } else {
        1.0
    };
    let x = (origin_x + target.x * scale_x).round() as i64;
    let y = (origin_y + target.y * scale_y).round() as i64;

    Ok(InputReport {
        backend: "niri+wlr-virtual-pointer+wtype",
        niri_window_id: niri_id,
        x,
        y,
        window_origin_x: origin_x,
        window_origin_y: origin_y,
        window_width: win_w,
        window_height: win_h,
        extent_width: output.width.round().max(1.0) as u32,
        extent_height: output.height.round().max(1.0) as u32,
    })
}

fn find_niri_window(win: &Rc<BrowserWindow>) -> Result<u64, String> {
    let pid = std::process::id() as u64;
    let title = win
        .window
        .title()
        .map(|s| s.to_string())
        .unwrap_or_default();
    let windows = niri_json(&["msg", "--json", "windows"])?;
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
        "could not uniquely match hwatu window in niri IPC (pid={pid}, title={title:?}, candidates={})",
        candidates.len()
    ))
}

fn niri_window_by_id(id: u64) -> Result<Value, String> {
    let windows = niri_json(&["msg", "--json", "windows"])?;
    for window in windows
        .as_array()
        .ok_or("niri windows response was not an array")?
    {
        if window.get("id").and_then(Value::as_u64) == Some(id) {
            return Ok(window.clone());
        }
    }
    Err(format!("niri window {id} disappeared after focus-window"))
}

fn workspace_output(workspace_id: u64) -> Result<String, String> {
    let workspaces = niri_json(&["msg", "--json", "workspaces"])?;
    for ws in workspaces
        .as_array()
        .ok_or("niri workspaces response was not an array")?
    {
        if ws.get("id").and_then(Value::as_u64) == Some(workspace_id) {
            return ws
                .get("output")
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| format!("workspace {workspace_id} had no output"));
        }
    }
    Err(format!("workspace {workspace_id} not found in niri IPC"))
}

#[derive(Debug, Clone)]
struct OutputGeometry {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

fn output_geometry(name: &str) -> Result<OutputGeometry, String> {
    let outputs = niri_json(&["msg", "--json", "outputs"])?;
    let logical = outputs
        .get(name)
        .and_then(|o| o.get("logical"))
        .ok_or_else(|| format!("niri output {name:?} had no logical geometry"))?;
    Ok(OutputGeometry {
        x: logical.get("x").and_then(Value::as_f64).unwrap_or(0.0),
        y: logical.get("y").and_then(Value::as_f64).unwrap_or(0.0),
        width: logical
            .get("width")
            .and_then(Value::as_f64)
            .ok_or_else(|| format!("niri output {name:?} had no logical width"))?,
        height: logical
            .get("height")
            .and_then(Value::as_f64)
            .ok_or_else(|| format!("niri output {name:?} had no logical height"))?,
    })
}

fn niri_json(args: &[&str]) -> Result<Value, String> {
    let out = Command::new("niri")
        .args(args)
        .output()
        .map_err(|e| format!("failed to run niri {}: {e}", args.join(" ")))?;
    if !out.status.success() {
        return Err(format!(
            "niri {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("niri {} returned invalid JSON: {e}", args.join(" ")))
}

fn pair_f64(v: &Value) -> Result<(f64, f64), String> {
    let arr = v.as_array().ok_or("expected JSON array pair")?;
    let x = arr
        .first()
        .and_then(Value::as_f64)
        .ok_or("expected pair[0] number")?;
    let y = arr
        .get(1)
        .and_then(Value::as_f64)
        .ok_or("expected pair[1] number")?;
    Ok((x, y))
}

#[cfg(test)]
mod tests {
    use super::RESOLVER_JS;

    #[test]
    fn resolver_is_all_frames_shape() {
        assert!(RESOLVER_JS.contains("__hwatuTrustedResolve"));
        assert!(RESOLVER_JS.contains("postMessage"));
        assert!(RESOLVER_JS.contains("iframe"));
    }
}
