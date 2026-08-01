//! Fake media playback for verification captures ("media shim").
//!
//! Video-gated pages break headless verification two ways on
//! WebKitGTK + GStreamer 1.28.5:
//!
//! 1. `HTMLMediaElement.play()/pause()` from page JS deadlocks the web
//!    process main thread inside
//!    `MediaPlayerPrivateGStreamer::changePipelineState` (observed on
//!    scale.com: a Lenis scroll handler calling `video.pause()` wedged
//!    every subsequent eval/shot forever).
//! 2. With autoplay denied (the stable mitigation for the load-time
//!    variant of the same wedge), `currentTime` never advances, so
//!    sites that gate content on real playback progress — e.g.
//!    scale.com's WebGL hero requires `readyState >= HAVE_FUTURE_DATA
//!    && currentTime >= 0.08` — never reveal the content under test.
//!    The capture then verifies a placeholder against a placeholder,
//!    a tautological pass.
//!
//! The shim patches `HTMLMediaElement.prototype` at document start,
//! before any page script runs:
//!
//! - `play()` resolves immediately and marks the element as playing in
//!   shim state; `pause()` clears it; `load()` is a no-op. GStreamer
//!   pipeline state is never touched by page JS.
//! - `paused`, `currentTime`, `readyState` (4), `ended` are shimmed
//!   accessors backed by that state. `currentTime` advances with
//!   `performance.now()` deltas inside a rAF loop, so it also advances
//!   correctly under `hwatu clock step` virtual time.
//! - The playback event contract fires: `loadedmetadata`, `loadeddata`,
//!   `canplay`, `canplaythrough`, `play`, `playing` on play, periodic
//!   `timeupdate` (4/s) while playing, `pause` on pause, `ended`+loop
//!   wrap at the (real, else fake 60s) duration.
//!
//! Trade-off: no decoded frames — video textures/paints stay black.
//! Geometry, uniforms, DOM animation, and gates driven by the JS
//! playback contract all run. That is exactly what scroll-animation
//! verification needs; pixel-perfect video content is out of scope.
//!
//! Gate: opt-in. `HWATU_MEDIA_SHIM=1` env, or `"media_shim": true` in
//! ~/.config/hwatu/config.json (env vars vanish on daemon restarts;
//! the config file persists — same lesson as the autoplay policy).

const MEDIA_SHIM_JS: &str = r#"(() => {
  'use strict';
  if (window.__hwatuMediaShim) return;
  // Top-level media documents (navigating straight to a .mp4/.mp3)
  // are WebKit's own <video> wrapper page: the media element IS the
  // content. Faking playback there breaks the only thing the page
  // does, so the shim must not apply.
  if (/^(video|audio)\//.test(document.contentType || '')) return;
  window.__hwatuMediaShim = true;

  const proto = HTMLMediaElement.prototype;
  const state = new WeakMap();
  const FAKE_DURATION = 60;

  const st = (el) => {
    let s = state.get(el);
    if (!s) {
      s = { playing: false, time: 0, started: false, lastTimeupdate: 0 };
      state.set(el, s);
    }
    return s;
  };

  const fire = (el, type) => {
    try { el.dispatchEvent(new Event(type)); } catch (e) {}
  };

  const realDuration = Object.getOwnPropertyDescriptor(proto, 'duration');
  const dur = (el) => {
    try {
      const d = realDuration && realDuration.get ? realDuration.get.call(el) : NaN;
      if (Number.isFinite(d) && d > 0) return d;
    } catch (e) {}
    return FAKE_DURATION;
  };

  proto.play = function () {
    const s = st(this);
    if (!s.playing) {
      s.playing = true;
      if (!s.started) {
        s.started = true;
        for (const t of ['loadedmetadata', 'loadeddata', 'canplay', 'canplaythrough']) fire(this, t);
      }
      fire(this, 'play');
      fire(this, 'playing');
    }
    return Promise.resolve();
  };
  proto.pause = function () {
    const s = st(this);
    if (s.playing) { s.playing = false; fire(this, 'pause'); }
  };
  proto.load = function () {};

  const def = (name, get, set) => {
    try {
      Object.defineProperty(proto, name, { configurable: true, get, set });
    } catch (e) {}
  };
  def('paused', function () { return !st(this).playing; });
  def('ended', function () { return false; });
  def('readyState', function () { return 4; });
  def('currentTime',
      function () { return st(this).time; },
      function (v) { st(this).time = Number(v) || 0; });

  // Advance playing elements with performance.now() deltas: rAF and
  // performance.now() both follow hwatu's virtual clock, so `clock
  // step` pumps deterministic playback for captures.
  const tracked = new Set();
  const collect = () => {
    let media;
    try { media = document.querySelectorAll('video, audio'); } catch (e) { return; }
    for (const m of media) tracked.add(m);
  };
  let last = performance.now();
  const tick = () => {
    const now = performance.now();
    const dt = Math.min(1, Math.max(0, (now - last) / 1000));
    last = now;
    for (const el of tracked) {
      const s = state.get(el);
      if (!s || !s.playing) continue;
      s.time += dt;
      const d = dur(el);
      if (s.time >= d) {
        if (el.loop) { s.time %= d; }
        else { s.time = d; s.playing = false; fire(el, 'ended'); }
      }
      if (now - s.lastTimeupdate >= 250) {
        s.lastTimeupdate = now;
        fire(el, 'timeupdate');
      }
    }
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
  setInterval(collect, 500);
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', collect, { once: true });
  } else {
    collect();
  }
})();"#;

/// Opt-in: `HWATU_MEDIA_SHIM=1|on|true` env, else `"media_shim": true`
/// in ~/.config/hwatu/config.json. Env wins when set at all (so `=0`
/// can override a config-enabled shim for one daemon run).
fn enabled() -> bool {
    match std::env::var("HWATU_MEDIA_SHIM").as_deref() {
        Ok("1") | Ok("on") | Ok("true") => return true,
        Ok(_) => return false,
        Err(_) => {}
    }
    config_media_shim().unwrap_or(false)
}

/// Read `"media_shim"` from ~/.config/hwatu/config.json.
fn config_media_shim() -> Option<bool> {
    let raw =
        std::fs::read_to_string(glib::user_config_dir().join("hwatu").join("config.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    v.get("media_shim")?.as_bool()
}

/// Inject the media shim into a WebView. Must run on every view
/// (prewarm pool and popups) before page content loads, same contract
/// as `console::wire_view` / `blurshield::wire_view`.
pub fn wire_view(view: &webkit6::WebView) {
    use webkit6::prelude::*;
    if !enabled() {
        return;
    }
    let Some(ucm) = view.user_content_manager() else {
        return;
    };
    let script = webkit6::UserScript::new(
        MEDIA_SHIM_JS,
        webkit6::UserContentInjectedFrames::AllFrames,
        webkit6::UserScriptInjectionTime::Start,
        &[],
        &[],
    );
    ucm.add_script(&script);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shim is opt-in: absent env and config keep it off, and the
    /// env value must dominate the config fallback in both directions.
    #[test]
    fn env_gate_semantics() {
        std::env::remove_var("HWATU_MEDIA_SHIM");
        // config may or may not exist on the test machine; only the
        // env-driven paths are deterministic here.
        std::env::set_var("HWATU_MEDIA_SHIM", "1");
        assert!(enabled());
        std::env::set_var("HWATU_MEDIA_SHIM", "0");
        assert!(!enabled());
        std::env::set_var("HWATU_MEDIA_SHIM", "off");
        assert!(!enabled());
        std::env::remove_var("HWATU_MEDIA_SHIM");
    }

    /// play() must fulfill the full "it really played" JS contract:
    /// promise resolution plus the event chain gates listen for.
    #[test]
    fn script_fires_playback_contract() {
        for ev in [
            "'loadedmetadata'",
            "'loadeddata'",
            "'canplay'",
            "'canplaythrough'",
            "'play'",
            "'playing'",
            "'timeupdate'",
            "'ended'",
        ] {
            assert!(MEDIA_SHIM_JS.contains(ev), "missing event {ev}");
        }
        assert!(MEDIA_SHIM_JS.contains("Promise.resolve()"));
    }

    /// The GStreamer-touching entry points must all be neutralized —
    /// pause() is the one that deadlocked scale.com captures.
    #[test]
    fn script_neutralizes_pipeline_entry_points() {
        assert!(MEDIA_SHIM_JS.contains("proto.play = function"));
        assert!(MEDIA_SHIM_JS.contains("proto.pause = function"));
        assert!(MEDIA_SHIM_JS.contains("proto.load = function () {}"));
    }

    /// currentTime must advance from performance.now() inside rAF so
    /// virtual-clock stepping (hwatu clock step) drives playback.
    #[test]
    fn script_advances_time_on_virtual_clock() {
        assert!(MEDIA_SHIM_JS.contains("performance.now()"));
        assert!(MEDIA_SHIM_JS.contains("requestAnimationFrame(tick)"));
        assert!(MEDIA_SHIM_JS.contains("readyState"));
    }

    /// Double injection (prewarm + navigation) must be a no-op.
    #[test]
    fn script_is_idempotent() {
        assert!(MEDIA_SHIM_JS.contains("__hwatuMediaShim"));
    }

    /// Top-level media documents (direct .mp4/.mp3 navigation) must be
    /// exempt: there the media element IS the page, and faking its
    /// playback breaks the only content.
    #[test]
    fn script_skips_media_documents() {
        assert!(MEDIA_SHIM_JS.contains("document.contentType"));
        assert!(MEDIA_SHIM_JS.contains("(video|audio)"));
    }
}
