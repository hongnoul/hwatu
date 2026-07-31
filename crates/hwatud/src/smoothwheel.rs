//! Chromium-curve wheel scrolling, injected as a user script.
//!
//! Why this exists: WebKitGTK's native smooth scrolling
//! (`ScrollAnimationSmooth`) animates each discrete wheel tick over
//! `min(distance/1000s, 200ms)` — ~100ms for a typical ~100px tick —
//! restarting from zero velocity with an ease-in-out every time. Ticks
//! arriving slower than the animation length (a human rolling the wheel
//! at a moderate pace) therefore produce isolated stop-start pulses
//! instead of a continuous glide. Chromium instead uses an
//! inverse-delta duration (small deltas animate *longer*, 100-200ms)
//! and, crucially, retargets a running animation with a cubic-bezier
//! whose initial slope preserves the current velocity
//! (`cc/animation/scroll_offset_animation_curve.cc`), so consecutive
//! ticks fuse into one continuous velocity curve. That difference is
//! measurable: on the same 16-tick input, WebKit's animator yields six
//! separate ~100ms pulses with dead gaps; Chromium yields one ~950ms
//! glide (see `scripts/bench-scroll/`).
//!
//! WebKit exposes no tunable for its curve, so hwatu re-implements the
//! Chromium curve in page JS: a capture-free `wheel` listener claims
//! *discrete* wheel ticks (integer deltas ≥ 24px, or line/page delta
//! modes), calls `preventDefault()`, and drives the scroller from
//! `requestAnimationFrame` with Chromium's exact constants. Precise
//! touchpad deltas (non-integer, small) are left to the engine, which
//! already handles them well (instant + kinetic).
//!
//! Fail-open by design: the handler bails — leaving native scrolling
//! intact — when the event is already `defaultPrevented`, not
//! cancelable, ctrl/alt/meta-modified (zoom and friends), or when no
//! ancestor can scroll in the delta direction. Any exception inside
//! the handler is swallowed before `preventDefault`, so a bug degrades
//! to engine-native scrolling, never to a dead wheel.
//!
//! `HWATU_SMOOTH_WHEEL=0` disables the whole thing.

/// See module docs. Constants mirror Chromium
/// `cc/animation/scroll_offset_animation_curve.cc` (kInverseDelta*)
/// and `ui/gfx/geometry/cubic_bezier` ease-in-out control points.
const SMOOTH_WHEEL_JS: &str = r#"(() => {
  'use strict';
  if (window.__hwatuSmoothWheel) return;
  window.__hwatuSmoothWheel = true;

  // Chromium kInverseDelta duration: frames-at-60fps as a function of
  // distance; small deltas animate longer so slow tick streams overlap.
  const RAMP_START = 120, RAMP_END = 480, MIN_DUR = 6, MAX_DUR = 12;
  const SLOPE = (MIN_DUR - MAX_DUR) / (RAMP_END - RAMP_START);
  const OFFSET = MAX_DUR - RAMP_START * SLOPE;
  const EPS = 0.01;

  function durationMs(delta) {
    const frames = Math.min(Math.max(OFFSET + Math.abs(delta) * SLOPE, MIN_DUR), MAX_DUR);
    return frames * 1000 / 60;
  }

  // Cubic bezier timing function (CSS semantics: x is time, y is
  // progress). Newton's method to invert x(s), like Blink/WebKit do.
  function bezier(x1, y1, x2, y2) {
    const sx = s => 3*s*(1-s)*(1-s)*x1 + 3*s*s*(1-s)*x2 + s*s*s;
    const sy = s => 3*s*(1-s)*(1-s)*y1 + 3*s*s*(1-s)*y2 + s*s*s;
    const dx = s => 3*(1-s)*(1-s)*x1 + 6*s*(1-s)*(x2-x1) + 3*s*s*(1-x2);
    const dy = s => 3*(1-s)*(1-s)*y1 + 6*s*(1-s)*(y2-y1) + 3*s*s*(1-y2);
    function solve(t) {
      let s = t;
      for (let i = 0; i < 8; i++) {
        const err = sx(s) - t, d = dx(s);
        if (Math.abs(err) < 1e-6 || Math.abs(d) < 1e-6) break;
        s -= err / d;
      }
      return Math.min(Math.max(s, 0), 1);
    }
    return {
      value(t) { return t <= 0 ? 0 : t >= 1 ? 1 : sy(solve(t)); },
      slope(t) { const s = solve(Math.min(Math.max(t, 0), 1)); const d = dx(s); return d ? dy(s) / d : 0; },
    };
  }

  const easeInOut = () => bezier(0.42, 0, 0.58, 1);
  // Chromium EaseInOutWithInitialSlope: scale the first control point
  // so the curve starts at the previous animation's velocity.
  function easeWithSlope(slope) {
    slope = Math.min(Math.max(slope, -1000), 1000);
    return bezier(0.42, 0.42 * slope, 0.58, 1);
  }

  // One live animation per scroller. Usually a single entry (the root).
  const anims = new Map();
  let rafId = 0;

  const maxScroll = (el, h) => h ? el.scrollWidth - el.clientWidth : el.scrollHeight - el.clientHeight;
  const getPos = (el, h) => h ? el.scrollLeft : el.scrollTop;
  function setPos(el, h, v) {
    // scrollTo with explicit behavior so a page-set
    // `scroll-behavior: smooth` can't stack its own animation on ours.
    el.scrollTo(h ? { left: v, behavior: 'instant' } : { top: v, behavior: 'instant' });
  }

  function tick(now) {
    rafId = 0;
    for (const [el, a] of anims) {
      const t = Math.min((now - a.start) / a.dur, 1);
      setPos(el, a.horiz, a.from + (a.to - a.from) * a.curve.value(t));
      if (t >= 1) anims.delete(el);
    }
    if (anims.size) rafId = requestAnimationFrame(tick);
  }
  const ensureRaf = () => { if (!rafId) rafId = requestAnimationFrame(tick); };

  function isScrollable(el, h) {
    if ((h ? el.scrollWidth - el.clientWidth : el.scrollHeight - el.clientHeight) <= 0) return false;
    const o = h ? getComputedStyle(el).overflowX : getComputedStyle(el).overflowY;
    return o === 'auto' || o === 'scroll';
  }

  // Nearest ancestor that can still move in `dir`. A scroller we're
  // already animating stays latched until its animation *target* (not
  // its current position) hits the extent, so a fast tick stream
  // doesn't leak into the parent mid-glide.
  function scrollTarget(node, h, dir) {
    const root = document.scrollingElement;
    let el = node instanceof Element ? node : root;
    for (; el; el = el.parentElement) {
      if (el !== root && !isScrollable(el, h)) continue;
      const a = anims.get(el);
      const eff = a && a.horiz === h ? a.to : getPos(el, h);
      if (dir > 0 ? eff < maxScroll(el, h) - 1 : eff > 1) return el;
    }
    return null;
  }

  // Discrete wheel ticks come from WebKitGTK as sparse integer deltas
  // (~100px). Precise touchpad deltas are frequent non-integer floats;
  // those keep the engine's native instant+kinetic path.
  function isDiscrete(e) {
    if (e.deltaMode !== 0) return true;
    return Number.isInteger(e.deltaY) && Number.isInteger(e.deltaX)
        && Math.abs(e.deltaY || e.deltaX) >= 24;
  }

  function animateBy(el, horiz, delta) {
    const now = performance.now();
    const max = maxScroll(el, horiz);
    const cur = getPos(el, horiz);
    let a = anims.get(el);
    if (a && a.horiz !== horiz) { anims.delete(el); a = null; }

    const target = Math.min(Math.max((a ? a.to : cur) + delta, 0), max);
    if (!a) {
      if (Math.abs(target - cur) < EPS) return;
      anims.set(el, { horiz, from: cur, to: target, start: now,
                      dur: durationMs(target - cur), curve: easeInOut() });
      ensureRaf();
      return;
    }

    // Chromium UpdateTarget: retarget from the current curve position,
    // preserving velocity via the initial-slope bezier, with the
    // velocity-based duration bound to avoid rubber-banding.
    const t = Math.min((now - a.start) / a.dur, 1);
    const pos = a.from + (a.to - a.from) * a.curve.value(t);
    const vel = a.curve.slope(t) * (a.to - a.from) / a.dur; // px/ms
    const newDelta = target - pos;
    if (Math.abs(newDelta) < EPS) { anims.delete(el); setPos(el, horiz, target); return; }

    let dur = durationMs(newDelta);
    if (Math.abs(vel) > EPS) {
      const bound = (newDelta / vel) * 2.5;
      if (bound >= 0) dur = Math.min(dur, bound);
    }
    if (dur < 1) { anims.delete(el); setPos(el, horiz, target); return; }

    a.from = pos; a.to = target; a.start = now; a.dur = dur;
    a.curve = easeWithSlope(vel * dur / newDelta);
    ensureRaf();
  }

  addEventListener('wheel', e => {
    try {
      if (e.defaultPrevented || !e.cancelable) return;
      if (e.ctrlKey || e.altKey || e.metaKey) return; // zoom & friends
      if (!isDiscrete(e)) return;

      let dx = e.deltaX, dy = e.deltaY;
      if (e.deltaMode === 1) { dx *= 40; dy *= 40; }
      else if (e.deltaMode === 2) { dx *= innerWidth; dy *= innerHeight; }
      if (e.shiftKey && !dx) { dx = dy; dy = 0; }

      const horiz = Math.abs(dx) > Math.abs(dy);
      const delta = horiz ? dx : dy;
      if (!delta) return;

      const el = scrollTarget(e.target, horiz, delta > 0 ? 1 : -1);
      if (!el) return; // nothing scrollable: stay native

      e.preventDefault();
      animateBy(el, horiz, delta);
    } catch (_) {
      // Fail open: native scrolling still works if we blow up.
    }
  }, { passive: false });

  // Other input takes over instantly: don't fight keys, drags, or
  // touches with a stale glide.
  for (const ev of ['keydown', 'mousedown', 'touchstart']) {
    addEventListener(ev, () => { anims.clear(); }, { capture: true, passive: true });
  }
})();"#;

/// True unless `HWATU_SMOOTH_WHEEL` explicitly disables the animator.
fn enabled() -> bool {
    !matches!(
        std::env::var("HWATU_SMOOTH_WHEEL").as_deref(),
        Ok("0") | Ok("off") | Ok("false")
    )
}

/// Inject the wheel animator into a WebView. Must run on every view
/// (prewarm pool and popups) before page content loads, same contract
/// as `console::wire_view`.
pub fn wire_view(view: &webkit6::WebView) {
    use webkit6::prelude::*;
    if !enabled() {
        return;
    }
    let Some(ucm) = view.user_content_manager() else {
        return;
    };
    let script = webkit6::UserScript::new(
        SMOOTH_WHEEL_JS,
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

    /// The env gate must treat only explicit "0"/"off"/"false" as off;
    /// absence and anything else keep the animator on.
    #[test]
    fn env_gate_semantics() {
        // Not set in the test env by default.
        std::env::remove_var("HWATU_SMOOTH_WHEEL");
        assert!(enabled());
        std::env::set_var("HWATU_SMOOTH_WHEEL", "0");
        assert!(!enabled());
        std::env::set_var("HWATU_SMOOTH_WHEEL", "off");
        assert!(!enabled());
        std::env::set_var("HWATU_SMOOTH_WHEEL", "1");
        assert!(enabled());
        std::env::remove_var("HWATU_SMOOTH_WHEEL");
    }

    /// The injected script must keep its fail-open shape: bail on
    /// non-cancelable/prevented events before any preventDefault.
    #[test]
    fn script_fails_open() {
        assert!(SMOOTH_WHEEL_JS.contains("e.defaultPrevented || !e.cancelable"));
        // preventDefault must appear after the discrete/scrollable
        // guards in the handler body.
        let handler = SMOOTH_WHEEL_JS
            .split("addEventListener('wheel'")
            .nth(1)
            .unwrap();
        let pd = handler.find("e.preventDefault()").unwrap();
        let discrete = handler.find("isDiscrete").unwrap();
        let target = handler.find("scrollTarget").unwrap();
        assert!(discrete < pd && target < pd);
    }
}
