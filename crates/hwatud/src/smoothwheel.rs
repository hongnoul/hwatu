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
//! Mandatory `scroll-snap` containers (YouTube Shorts, TikTok-style
//! feeds) get native-app paging instead of the delta glide: one
//! discrete tick animates to exactly the next snap point, ticks
//! landing mid-flight are absorbed so a fast wheel can't skip pages,
//! and a late or reverse tick retargets one page further while
//! preserving velocity. Feeds that page from JS without CSS snap
//! (Instagram Reels mobile web) are caught by a card heuristic: a
//! scroller whose direct children are uniform viewport-sized cards
//! pages on card boundaries the same way. A proportional glide on
//! these feeds either fights the page's own snap resolution or
//! strands the view between reels; paging is what the equivalent
//! native apps do.
//!
//! Keyboard scrolling (Arrow/Page/Space) goes through the same
//! animator, because WebKit's keydown scroll has the identical
//! stop-start pulse problem and is what makes arrow-keying through a
//! reel feed feel mushy compared to a native app. Arrows/PageUp/Down/
//! Space page snap containers exactly like a wheel tick, and glide
//! plain scrollers with the Chromium curve (key auto-repeat retargets
//! the running animation, fusing a held key into one accelerating
//! glide). The handler bails on editable/interactive targets,
//! modifier chords, and `defaultPrevented` events, so pages that own
//! their keys (players, games, editors) keep them.
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

  // Snap paging: one wheel tick = one snap point, like native reel
  // feeds. Fixed duration (native apps page at a steady cadence) and
  // an absorb window so the several ticks of one physical flick move
  // exactly one page.
  const SNAP_DUR = 350, SNAP_ABSORB = 0.55, SNAP_CACHE_MS = 300;
  const snapCache = new WeakMap();

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

  // WebKitGTK re-snaps every programmatic scroll on a snap container
  // in the same frame, quantizing our per-rAF positions to snap points
  // and collapsing the animation into a teleport. Suspend the
  // container's snap-type while an animation is in flight; the
  // animation always ends on an exact snap offset, so restoring is a
  // re-snap no-op.
  function suspendSnap(el) {
    const nodes = el === document.scrollingElement
      ? [document.documentElement, document.body] : [el];
    const saved = [];
    for (const n of nodes) {
      if (!n) continue;
      const st = getComputedStyle(n).scrollSnapType;
      if (!st || st === 'none') continue;
      saved.push([n, n.style.scrollSnapType]);
      n.style.scrollSnapType = 'none';
    }
    return saved;
  }
  function endAnim(el, a) {
    anims.delete(el);
    if (a && a.saved) for (const [n, v] of a.saved) n.style.scrollSnapType = v;
  }
  function clearAnims() {
    for (const [el, a] of anims) endAnim(el, a);
  }

  function tick(now) {
    rafId = 0;
    for (const [el, a] of anims) {
      const t = Math.min((now - a.start) / a.dur, 1);
      setPos(el, a.horiz, a.from + (a.to - a.from) * a.curve.value(t));
      if (t >= 1) endAnim(el, a);
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

  // True when `el` snaps mandatorily on this axis. The root scroller's
  // snap type may live on <html> or <body>.
  function isMandatorySnap(el, h) {
    const nodes = el === document.scrollingElement
      ? [document.documentElement, document.body]
      : [el];
    for (const n of nodes) {
      if (!n) continue;
      const st = getComputedStyle(n).scrollSnapType || 'none';
      if (!st.includes('mandatory')) continue;
      const axis = st.split(' ')[0];
      if (axis === 'both') return true;
      if (h ? (axis === 'x' || axis === 'inline') : (axis === 'y' || axis === 'block')) return true;
    }
    return false;
  }

  // Content offsets of snap-aligned descendants (honoring
  // start/center/end alignment), sorted and deduped. Depth-capped
  // walk, memoized briefly so rapid ticks don't re-measure a
  // virtualized feed every event.
  function snapOffsets(el, h) {
    const now = performance.now();
    const c = snapCache.get(el);
    if (c && c.h === h && now - c.t < SNAP_CACHE_MS) return c.offsets;
    const scope = el === document.scrollingElement ? document.documentElement : el;
    const base = scope.getBoundingClientRect();
    const pos = getPos(el, h);
    const max = maxScroll(el, h);
    const port = h ? el.clientWidth : el.clientHeight;
    const raw = [];
    const stack = [[scope, 0]];
    let seen = 0;
    while (stack.length && seen < 600) {
      const [node, d] = stack.pop();
      for (const ch of node.children) {
        if (++seen > 600) break;
        const align = getComputedStyle(ch).scrollSnapAlign;
        if (align !== 'none') {
          const r = ch.getBoundingClientRect();
          // Two-value form is `<block> <inline>`; vertical scrolling
          // uses the block component, horizontal the inline one.
          const parts = align.split(' ');
          const ax = parts.length > 1 ? (h ? parts[1] : parts[0]) : parts[0];
          let off = pos + (h ? r.left - base.left : r.top - base.top);
          const size = h ? r.width : r.height;
          if (ax === 'center') off -= (port - size) / 2;
          else if (ax === 'end') off -= port - size;
          raw.push(Math.min(Math.max(off, 0), max));
        } else if (d < 3) {
          stack.push([ch, d + 1]);
        }
      }
    }
    raw.sort((a, b) => a - b);
    const offsets = [];
    for (const v of raw) {
      if (!offsets.length || v - offsets[offsets.length - 1] > 1) offsets.push(v);
    }
    snapCache.set(el, { t: now, h, offsets });
    return offsets;
  }

  // Feeds that page without CSS snap (Instagram Reels mobile web,
  // TikTok-style feeds drive paging from touch JS instead): a large
  // scroller whose direct children are uniform, viewport-sized cards.
  // Card tops are the page boundaries. The uniformity requirement
  // (>=80% of children sized ~= the port) keeps ordinary documents,
  // whose child heights vary, out of the paging path.
  const feedCache = new WeakMap();
  function feedOffsets(el, h) {
    const now = performance.now();
    const c = feedCache.get(el);
    if (c && c.h === h && now - c.t < SNAP_CACHE_MS) return c.offsets;
    let offsets = null;
    const port = h ? el.clientWidth : el.clientHeight;
    const kids = el === document.scrollingElement ? [] : el.children;
    if (port >= 300 && kids.length >= 2) {
      const base = el.getBoundingClientRect();
      const pos = getPos(el, h);
      const max = maxScroll(el, h);
      const raw = [];
      let uniform = 0, measured = 0;
      for (const ch of kids) {
        if (measured >= 40) break;
        const r = ch.getBoundingClientRect();
        const size = h ? r.width : r.height;
        if (size < 1) continue;
        measured++;
        if (Math.abs(size - port) <= Math.max(8, port * 0.02)) uniform++;
        raw.push(Math.min(Math.max(pos + (h ? r.left - base.left : r.top - base.top), 0), max));
      }
      if (measured >= 2 && uniform / measured >= 0.8) {
        raw.sort((a, b) => a - b);
        offsets = [];
        for (const v of raw) {
          if (!offsets.length || v - offsets[offsets.length - 1] > 1) offsets.push(v);
        }
        if (offsets.length < 2) offsets = null;
      }
    }
    feedCache.set(el, { t: now, h, offsets });
    return offsets;
  }

  // Animate to an absolute snap offset over a fixed duration,
  // preserving in-flight velocity on retarget.
  function animateTo(el, horiz, target) {
    const now = performance.now();
    const cur = getPos(el, horiz);
    let a = anims.get(el);
    if (a && a.horiz !== horiz) { endAnim(el, a); a = null; }
    if (!a) {
      if (Math.abs(target - cur) < EPS) return;
      anims.set(el, { horiz, from: cur, to: target, start: now,
                      dur: SNAP_DUR, curve: easeInOut(), snap: true,
                      saved: suspendSnap(el) });
      ensureRaf();
      return;
    }
    const t = Math.min((now - a.start) / a.dur, 1);
    const pos = a.from + (a.to - a.from) * a.curve.value(t);
    const vel = a.curve.slope(t) * (a.to - a.from) / a.dur;
    const newDelta = target - pos;
    if (Math.abs(newDelta) < EPS) { setPos(el, horiz, target); endAnim(el, a); return; }
    a.from = pos; a.to = target; a.start = now; a.dur = SNAP_DUR; a.snap = true;
    a.curve = Math.abs(vel) > EPS ? easeWithSlope(vel * SNAP_DUR / newDelta) : easeInOut();
    ensureRaf();
  }

  // Page a snap or card-feed scroller by one page. Returns true if
  // the tick was consumed (paged, absorbed mid-flight, or at extent).
  function pageSnap(el, horiz, dir) {
    // While our animation flies, the container's snap-type is
    // suspended, so check the live animation before computed style.
    const a = anims.get(el);
    const snapInFlight = !!(a && a.snap && a.horiz === horiz);
    if (snapInFlight) {
      const t = (performance.now() - a.start) / a.dur;
      const going = a.to > a.from ? 1 : -1;
      // A flick emits several ticks: absorb same-direction ticks
      // early in flight so one flick moves exactly one page.
      if (going === dir && t < SNAP_ABSORB) return true;
    }
    // Mandatory CSS snap first; fall back to the uniform-card feed
    // heuristic (Reels mobile web pages by JS, not CSS snap).
    let offsets = isMandatorySnap(el, horiz) ? snapOffsets(el, horiz) : null;
    if (!offsets || offsets.length < 2) offsets = feedOffsets(el, horiz);
    if (!offsets || offsets.length < 2) return false;
    const eff = a && a.horiz === horiz ? a.to : getPos(el, horiz);
    let target = null;
    if (dir > 0) {
      for (const v of offsets) { if (v > eff + 1) { target = v; break; } }
    } else {
      for (let i = offsets.length - 1; i >= 0; i--) {
        if (offsets[i] < eff - 1) { target = offsets[i]; break; }
      }
    }
    // At the feed's extent: consume the tick rather than leaking a
    // glide into an ancestor mid-feed.
    if (target == null) return true;
    animateTo(el, horiz, target);
    return true;
  }

  function animateBy(el, horiz, delta) {
    const now = performance.now();
    const max = maxScroll(el, horiz);
    const cur = getPos(el, horiz);
    let a = anims.get(el);
    if (a && a.horiz !== horiz) { endAnim(el, a); a = null; }

    const target = Math.min(Math.max((a ? a.to : cur) + delta, 0), max);
    if (!a) {
      if (Math.abs(target - cur) < EPS) return;
      anims.set(el, { horiz, from: cur, to: target, start: now,
                      dur: durationMs(target - cur), curve: easeInOut(),
                      saved: suspendSnap(el) });
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
    if (Math.abs(newDelta) < EPS) { setPos(el, horiz, target); endAnim(el, a); return; }

    let dur = durationMs(newDelta);
    if (Math.abs(vel) > EPS) {
      const bound = (newDelta / vel) * 2.5;
      if (bound >= 0) dur = Math.min(dur, bound);
    }
    if (dur < 1) { setPos(el, horiz, target); endAnim(el, a); return; }

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

      // Snap paging decides before preventDefault so an exception in
      // it degrades to native scrolling, not a dead wheel.
      if (pageSnap(el, horiz, delta > 0 ? 1 : -1)) { e.preventDefault(); return; }
      e.preventDefault();
      animateBy(el, horiz, delta);
    } catch (_) {
      // Fail open: native scrolling still works if we blow up.
    }
  }, { passive: false });

  // Keyboard scrolling through the same animator. WebKit's keydown
  // scroller has the identical isolated-pulse problem as its wheel
  // animator, and on mandatory-snap feeds native key scroll strands
  // the view between snap points; paging is what native apps do.
  const KEY_LINE = 40; // px per arrow tap, Chromium's line step
  const PAGE_FRACTION = 0.85;
  function keyDelta(e) {
    switch (e.key) {
      case 'ArrowDown':  return { h: false, dir:  1, page: false };
      case 'ArrowUp':    return { h: false, dir: -1, page: false };
      case 'ArrowRight': return { h: true,  dir:  1, page: false };
      case 'ArrowLeft':  return { h: true,  dir: -1, page: false };
      case 'PageDown':   return { h: false, dir:  1, page: true };
      case 'PageUp':     return { h: false, dir: -1, page: true };
      case ' ':          return { h: false, dir: e.shiftKey ? -1 : 1, page: true };
      default: return null;
    }
  }
  // Targets that own their keys: never steal from text entry or
  // controls. Interactive-widget roles keep arrows too.
  function ownsKeys(t) {
    if (!(t instanceof Element)) return false;
    if (t.isContentEditable) return true;
    return t.closest(
      'input,textarea,select,button,video,audio,[contenteditable=""],[contenteditable="true"],' +
      '[role="listbox"],[role="menu"],[role="slider"],[role="textbox"],[role="combobox"]'
    ) != null;
  }

  // Bubble phase on window: the page's own handlers (players, games,
  // editors) run first, and their preventDefault makes us bail. A
  // stopPropagation upstream degrades to engine-native key scroll.
  addEventListener('keydown', e => {
    try {
      const k = keyDelta(e);
      // Non-scroll keys (or chords) may still move the view natively
      // (Home/End, page shortcuts): drop any stale glide.
      if (!k || e.ctrlKey || e.altKey || e.metaKey
          || (e.shiftKey && e.key !== ' ')) { clearAnims(); return; }
      if (e.defaultPrevented || !e.cancelable) return;
      if (ownsKeys(e.target)) { clearAnims(); return; }

      // Keydown targets the focused element (often <body>), not the
      // element under the pointer like wheel does. When the walk from
      // focus finds nothing, retry from the viewport center so inner
      // feed scrollers (reels with body focus) are still reached.
      let el = scrollTarget(e.target, k.h, k.dir);
      if (!el) {
        const mid = document.elementFromPoint(innerWidth / 2, innerHeight / 2);
        if (mid) el = scrollTarget(mid, k.h, k.dir);
      }
      if (!el) return; // nothing scrollable: stay native

      if (pageSnap(el, k.h, k.dir)) { e.preventDefault(); return; }
      const port = k.h ? el.clientWidth : el.clientHeight;
      const delta = k.dir * (k.page ? port * PAGE_FRACTION : KEY_LINE);
      e.preventDefault();
      animateBy(el, k.h, delta);
    } catch (_) {
      // Fail open: native key scrolling still works if we blow up.
    }
  }, { passive: false });

  // Other input takes over instantly: don't fight drags or touches
  // with a stale glide. (Keys are handled above.)
  for (const ev of ['mousedown', 'touchstart']) {
    addEventListener(ev, () => { clearAnims(); }, { capture: true, passive: true });
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

    /// Snap paging must claim mandatory-snap feeds (Reels-style) and
    /// must decide before any preventDefault so an exception in it
    /// falls back to native scrolling.
    #[test]
    fn snap_paging_shape() {
        assert!(SMOOTH_WHEEL_JS.contains("scrollSnapType"));
        assert!(SMOOTH_WHEEL_JS.contains("mandatory"));
        assert!(SMOOTH_WHEEL_JS.contains("scrollSnapAlign"));
        let handler = SMOOTH_WHEEL_JS
            .split("addEventListener('wheel'")
            .nth(1)
            .unwrap();
        let snap = handler.find("pageSnap").unwrap();
        let pd = handler.find("e.preventDefault()").unwrap();
        assert!(snap < pd, "pageSnap must run before preventDefault");
    }

    /// The keyboard path must guard editable/interactive targets and
    /// defaultPrevented before any preventDefault, and must page snap
    /// containers before gliding — same fail-open contract as wheel.
    #[test]
    fn key_handler_shape() {
        let handler = SMOOTH_WHEEL_JS
            .split("addEventListener('keydown'")
            .nth(1)
            .expect("keydown handler present");
        let pd = handler.find("e.preventDefault()").unwrap();
        let owns = handler.find("ownsKeys").unwrap();
        let prevented = handler.find("e.defaultPrevented").unwrap();
        let snap = handler.find("pageSnap").unwrap();
        assert!(owns < pd, "ownsKeys guard must precede preventDefault");
        assert!(prevented < pd, "defaultPrevented check must precede preventDefault");
        assert!(snap < pd, "pageSnap must run before glide preventDefault");
        // Bubble phase, not capture: page handlers keep priority.
        assert!(!handler.starts_with(", e => {}, { capture: true"));
        for key in ["ArrowDown", "ArrowUp", "PageDown", "PageUp"] {
            assert!(SMOOTH_WHEEL_JS.contains(key), "{key} handled");
        }
    }

    /// Snapless card feeds (Reels mobile web) must be caught by the
    /// uniform-card heuristic, and the heuristic must require uniform
    /// viewport-sized children so ordinary documents never page.
    #[test]
    fn card_feed_heuristic_shape() {
        assert!(SMOOTH_WHEEL_JS.contains("feedOffsets"));
        // Uniformity gate present (>= 80% of measured children).
        assert!(SMOOTH_WHEEL_JS.contains("uniform / measured >= 0.8"));
        // Fallback order inside pageSnap: CSS snap first, then feed.
        let ps = SMOOTH_WHEEL_JS.split("function pageSnap").nth(1).unwrap();
        let css = ps.find("isMandatorySnap").unwrap();
        let feed = ps.find("feedOffsets").unwrap();
        assert!(css < feed, "CSS snap must be preferred over the heuristic");
    }
}
