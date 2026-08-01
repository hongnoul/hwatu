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
//! already handles them well (instant + kinetic) — except on the
//! Instagram gesture feed, see below.
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
//! native apps do. Instagram's feed goes one step further: its
//! current-reel state advances only from touch gestures, so paging it
//! by scrollTop freezes the URL and pagination (the feed "runs out"
//! after the seeded batch); there each page tick dispatches a
//! synthetic pointer swipe instead (see `swipeFeed` in the script).
//! Precise touchpad deltas on that feed are claimed for the same
//! reason: the engine's native precise scroll moves pixels without
//! advancing IG's gesture state, so accumulated two-finger flicks
//! are translated into the same synthetic swipe (`preciseFeedScroll`).
//!
//! Paging also performs the native clients' commit-time playback
//! handoff: the moment a page tick commits, the incoming card's video
//! starts and the outgoing video's audio ramps out over ~40ms and
//! pauses. Web feeds otherwise wait for their IntersectionObserver
//! after the scroll settles, which reads as a dead gap between reels
//! (or, on the synthetic-swipe path, as the old reel's audio playing
//! over the transition). Deliberately no crossfade: the native apps
//! hard-cut audio at gesture commit too, and overlapping reel audio
//! is universally perceived as a bug.
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
//! Shortform feeds (Instagram Reels, YouTube Shorts, TikTok) share one
//! unified control scheme layered ahead of the generic key scrolling,
//! modeled on the Instagram Reels native app:
//!   - ArrowUp/ArrowDown: snap-scroll one video (via the snap pager);
//!   - ArrowRight held: the visible video plays at 2x, restoring its
//!     prior rate on release;
//!   - ArrowLeft: toggles the comment section — clicks the comment
//!     control when the sheet is closed and the sheet's close control
//!     when it is open, rather than assuming the comment control
//!     itself is a toggle;
//!   - Space: toggles play/pause on the visible video.
//!
//! The shortcuts are scoped to shortform URLs and fail open when no
//! visible media or matching control can be found; sites that handle
//! a key themselves (and preventDefault it) keep priority.
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

  // Commit-time playback handoff, the native clients' trick: the
  // incoming reel starts playing the moment the page gesture commits
  // (under the still-running transition), and the outgoing reel's
  // audio ramps out and pauses. Web feeds only play/pause from an
  // IntersectionObserver after the scroll settles, which reads as a
  // dead gap — or, through the synthetic-swipe path, as the old
  // reel's audio running over the whole transition. Deliberately no
  // crossfade: native hard-cuts at commit too (the ~40ms ramp only
  // strips the click of an abrupt cut), and both actions are
  // idempotent with the site's own later observer work.
  const RAMP_OUT_MS = 40, RAMP_IN_MS = 80;
  const ramping = new WeakSet();
  function rampVolume(v, to, ms, then) {
    if (ramping.has(v)) return;
    ramping.add(v);
    const from = v.volume, t0 = performance.now();
    function step(now) {
      const t = Math.min((now - t0) / ms, 1);
      try { v.volume = from + (to - from) * t; } catch (_) {}
      if (t < 1) requestAnimationFrame(step);
      else { ramping.delete(v); if (then) then(); }
    }
    requestAnimationFrame(step);
  }
  function handoffPlayback(dir) {
    if (!shortformSite()) return;
    const out = activeShortformVideo();
    // The incoming card is still ~one viewport away in the paging
    // direction at commit time: nearest video at least 40% of a
    // viewport from center. None found (virtualized card not mounted
    // yet) fails open — the site's observer handles it as before.
    const mid = innerHeight / 2;
    let inc = null, best = Infinity;
    for (const v of document.querySelectorAll('video')) {
      if (v === out) continue;
      const r = v.getBoundingClientRect();
      if (r.width < 1 || r.height < 1) continue;
      const d = ((r.top + r.bottom) / 2 - mid) * dir;
      if (d < innerHeight * 0.4) continue;
      if (d < best) { best = d; inc = v; }
    }
    if (inc && inc.paused && !ramping.has(inc)) {
      const iv = inc.volume;
      try { inc.volume = 0; } catch (_) {}
      const p = inc.play();
      if (p && p.catch) p.catch(() => {});
      rampVolume(inc, iv, RAMP_IN_MS);
    }
    if (out && !out.paused) {
      const ov = out.volume;
      rampVolume(out, 0, RAMP_OUT_MS, () => {
        // Restore the element's own volume after the pause so a
        // scroll-back (or the site recycling the element) never
        // inherits a silenced video.
        try { out.pause(); out.volume = ov; } catch (_) {}
      });
    }
  }

  // Instagram mobile-web Reels ignore programmatic scrolling: the
  // current-reel index lives in gesture state (React pointer handlers
  // on the feed element), so setting scrollTop moves pixels while the
  // URL, active video, and next-batch pagination stay frozen — the
  // feed "runs out" after the seeded ~6 reels. The one input the feed
  // does advance on is a touch swipe, so page it with a synthetic
  // pointer gesture. Verified live: each swipe updates the reel URL
  // and the graphql pagination fetch fires (children grew 7 -> 21).
  const SWIPE_ID = 0x5157, SWIPE_STEPS = 10, SWIPE_STEP_MS = 16;
  const SWIPE_ABSORB_MS = 650; // gesture + the site's own snap animation
  let swipeUntil = 0;
  let capGuarded = false;
  // Capturing a pointer that doesn't exist throws NotFoundError, which
  // would abort the site's pointerdown handler mid-gesture. Swallow
  // the failure for our synthetic pointer id only; real pointers keep
  // real errors. Installed lazily so non-feed pages stay untouched.
  function guardPointerCapture() {
    if (capGuarded) return;
    capGuarded = true;
    const orig = Element.prototype.setPointerCapture;
    Element.prototype.setPointerCapture = function (id) {
      try { return orig.call(this, id); }
      catch (err) { if (id !== SWIPE_ID) throw err; }
    };
  }
  function swipeFeed(el, dir) {
    const now = performance.now();
    if (now < swipeUntil) return true; // absorb the rest of the flick
    swipeUntil = now + SWIPE_STEPS * SWIPE_STEP_MS + SWIPE_ABSORB_MS;
    guardPointerCapture();
    const a = anims.get(el);
    if (a) endAnim(el, a); // a stale glide must not fight the gesture
    const r = el.getBoundingClientRect();
    const cx = r.left + r.width / 2;
    const [y0, y1] = dir > 0
      ? [r.top + r.height * 0.80, r.top + r.height * 0.15]
      : [r.top + r.height * 0.15, r.top + r.height * 0.80];
    const mk = (type, y) => new PointerEvent(type, {
      bubbles: true, cancelable: true, composed: true,
      pointerId: SWIPE_ID, pointerType: 'touch', isPrimary: true,
      clientX: cx, clientY: y,
      buttons: type === 'pointerup' ? 0 : 1,
      pressure: type === 'pointerup' ? 0 : 0.5, view: window,
    });
    el.dispatchEvent(mk('pointerdown', y0));
    let i = 0;
    const iv = setInterval(() => {
      try {
        i++;
        el.dispatchEvent(mk('pointermove', y0 + (y1 - y0) * i / SWIPE_STEPS));
        if (i >= SWIPE_STEPS) {
          clearInterval(iv);
          el.dispatchEvent(mk('pointerup', y1));
          // Gesture commit: start the incoming reel's playback now
          // instead of waiting out IG's settle + observer.
          handoffPlayback(dir);
        }
      } catch (_) {
        clearInterval(iv);
      }
    }, SWIPE_STEP_MS);
    return true;
  }

  // The IG feed sometimes reports no scrollable extent at all (its
  // virtualization positions cards purely with transforms, which don't
  // grow scrollHeight), so the scrollTarget walk comes up empty while
  // a swipeable feed fills the viewport. Find it by the card
  // heuristic alone: walk up from the start element, then from the
  // viewport center (keydown targets <body>, which is above the feed).
  function gestureFeed(start) {
    if (shortformSite() !== 'instagram') return null;
    const roots = [start instanceof Element ? start : null,
                   document.elementFromPoint(innerWidth / 2, innerHeight / 2)];
    for (const root of roots) {
      for (let el = root; el; el = el.parentElement) {
        if (el !== document.scrollingElement && el.children.length >= 2
            && feedOffsets(el, false)) return el;
      }
    }
    return null;
  }

  // Precise touchpad deltas on the IG feed must be claimed too: the
  // engine's native precise scroll moves pixels without advancing
  // IG's gesture-held reel index, silently desyncing the feed (URL,
  // active video, and pagination freeze — the same failure scrollTop
  // paging has). Accumulate the fine deltas and page via the same
  // synthetic swipe once a flick's worth arrives. Returns true when
  // the event belongs to the feed (caller preventDefaults even while
  // accumulating: leaked pixels are the desync).
  const PRECISE_TRIGGER = 120, PRECISE_IDLE_MS = 300;
  let preciseAcc = 0, preciseLast = 0;
  function preciseFeedScroll(target, dy) {
    if (shortformSite() !== 'instagram') return false;
    const feed = gestureFeed(target);
    if (!feed) return false;
    const now = performance.now();
    if (now - preciseLast > PRECISE_IDLE_MS) preciseAcc = 0;
    preciseLast = now;
    if (now < swipeUntil) return true; // mid-swipe: absorb the flick
    preciseAcc += dy;
    if (Math.abs(preciseAcc) >= PRECISE_TRIGGER) {
      const dir = preciseAcc > 0 ? 1 : -1;
      preciseAcc = 0;
      swipeFeed(feed, dir);
    }
    return true;
  }

  // Page a snap or card-feed scroller by one page. Returns true if
  // the tick was consumed (paged, absorbed mid-flight, or at extent).
  function pageSnap(el, horiz, dir) {
    // Gesture-driven feeds first: scrollTop paging silently breaks
    // them (see swipeFeed). The uniform-card check keeps the comments
    // sheet and other ordinary scrollers on the normal path. When the
    // walk latched onto a small wrapper scroller instead of the card
    // feed itself, gestureFeed relocates the real feed.
    if (!horiz && shortformSite() === 'instagram') {
      const feed = feedOffsets(el, false) ? el : gestureFeed(el);
      if (feed) return swipeFeed(feed, dir);
    }
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
    // Page commit on a snap/card feed: same handoff as the swipe
    // path. handoffPlayback gates itself to shortform sites.
    if (!horiz) handoffPlayback(dir);
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
      if (!isDiscrete(e)) {
        // Precise touchpad deltas normally stay native, but on the
        // IG gesture feed native pixels desync the reel state: claim
        // and translate them into synthetic swipes.
        if (!e.shiftKey && e.deltaY
            && preciseFeedScroll(e.target, e.deltaY)) e.preventDefault();
        return;
      }

      let dx = e.deltaX, dy = e.deltaY;
      if (e.deltaMode === 1) { dx *= 40; dy *= 40; }
      else if (e.deltaMode === 2) { dx *= innerWidth; dy *= innerHeight; }
      if (e.shiftKey && !dx) { dx = dy; dy = 0; }

      const horiz = Math.abs(dx) > Math.abs(dy);
      const delta = horiz ? dx : dy;
      if (!delta) return;

      const el = scrollTarget(e.target, horiz, delta > 0 ? 1 : -1);
      if (!el) {
        // IG's transform-virtualized feed can be unscrollable by
        // extent (see gestureFeed) while still swipeable.
        const feed = !horiz && gestureFeed(e.target);
        if (feed && swipeFeed(feed, delta > 0 ? 1 : -1)) e.preventDefault();
        return; // nothing scrollable: stay native
      }

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

  // Shortform feeds (Instagram Reels, YouTube Shorts, TikTok) share
  // one unified control scheme. Controls are rendered as anonymous
  // role=button wrappers around SVGs on these sites, so matching uses
  // stable accessibility labels (plus TikTok's data-e2e hooks) rather
  // than generated class names. The gate is URL-scoped so
  // ArrowLeft/ArrowRight/Space retain their normal behavior elsewhere.
  function shortformSite() {
    const host = location.hostname.toLowerCase().replace(/^www\./, '');
    const inHost = s => host === s || host.endsWith('.' + s);
    const path = location.pathname;
    if (inHost('instagram.com') && /(^|\/)reels?(\/|$)/i.test(path)) return 'instagram';
    if (inHost('youtube.com') && /^\/shorts(\/|$)/i.test(path)) return 'youtube';
    if (inHost('tiktok.com')
        && /^\/(?:$|foryou|following|friends|explore|@[^/]+\/video\/)/i.test(path)) return 'tiktok';
    return null;
  }
  function visibleRect(r) {
    return r.width > 0 && r.height > 0 && r.bottom > 0 && r.right > 0
      && r.left < innerWidth && r.top < innerHeight;
  }
  function activeShortformVideo() {
    let best = null, bestArea = 0;
    for (const v of document.querySelectorAll('video')) {
      const r = v.getBoundingClientRect();
      if (!visibleRect(r)) continue;
      const w = Math.min(r.right, innerWidth) - Math.max(r.left, 0);
      const h = Math.min(r.bottom, innerHeight) - Math.max(r.top, 0);
      const area = Math.max(0, w) * Math.max(0, h);
      if (area > bestArea) { best = v; bestArea = area; }
    }
    return best;
  }
  const speedRestore = new Map();
  function releaseShortformSpeed() {
    for (const [v, rate] of speedRestore) {
      try { v.playbackRate = rate; } catch (_) {}
    }
    speedRestore.clear();
  }
  function holdShortformSpeed() {
    const v = activeShortformVideo();
    if (!v) return false;
    if (!speedRestore.has(v)) speedRestore.set(v, v.playbackRate);
    v.playbackRate = 2;
    return true;
  }
  // Space toggles play/pause on the visible video, like the native
  // apps. The play() promise rejection (autoplay policy) is swallowed.
  function toggleShortformPlayback() {
    const v = activeShortformVideo();
    if (!v) return false;
    if (v.paused) { const p = v.play(); if (p && p.catch) p.catch(() => {}); }
    else v.pause();
    return true;
  }
  // Of several matching controls (one per stacked video card), pick the
  // one nearest the active video: that is the card the user is watching.
  function nearestButton(buttons) {
    if (!buttons.length) return null;
    const active = activeShortformVideo();
    const ar = active && active.getBoundingClientRect();
    const ax = ar ? (ar.left + ar.right) / 2 : innerWidth / 2;
    const ay = ar ? (ar.top + ar.bottom) / 2 : innerHeight / 2;
    return buttons.reduce((best, b) => {
      const r = b.getBoundingClientRect();
      const score = Math.hypot((r.left + r.right) / 2 - ax,
                               (r.top + r.bottom) / 2 - ay);
      return !best || score < best.score ? { b, score } : best;
    }, null).b;
  }
  function commentButton() {
    const selector = 'svg[aria-label="Comment" i],svg[aria-label*="comment" i]';
    const match = '[aria-label*="comment" i],[data-e2e*="comment-icon"]';
    const buttons = [];
    for (const b of document.querySelectorAll('button,[role="button"]')) {
      const r = b.getBoundingClientRect();
      if (visibleRect(r) && (b.matches(match) || b.querySelector(selector))) {
        buttons.push(b);
      }
    }
    return nearestButton(buttons);
  }
  function commentCloseButton() {
    const buttons = [];
    // Search only inside comment-sheet roots so unrelated page close
    // buttons cannot steal ArrowLeft: the open sheet is a dialog on
    // Instagram/TikTok mobile web, an engagement panel on YouTube.
    const roots = '[role="dialog"],[aria-modal="true"],' +
      'ytd-engagement-panel-section-list-renderer,[data-e2e="comment-panel"]';
    for (const root of document.querySelectorAll(roots)) {
      for (const b of root.querySelectorAll('button,[role="button"]')) {
        const r = b.getBoundingClientRect();
        if (visibleRect(r) && (b.matches('[aria-label*="close" i],[data-e2e*="close" i]')
            || b.querySelector('svg[aria-label*="close" i]'))) {
          buttons.push(b);
        }
      }
    }
    return nearestButton(buttons);
  }
  // React (Instagram) attaches its tap handler to the pointer-event
  // contract, not the click event: a bare el.click() on the comment
  // control is silently ignored. Press controls with the full
  // hover+press sequence aimed at the hit-test point inside the
  // control; that satisfies React, YouTube's polymer buttons, and
  // TikTok alike.
  function buttonTarget(b) {
    if (!b) return null;
    const r = b.getBoundingClientRect();
    const x = (r.left + r.right) / 2, y = (r.top + r.bottom) / 2;
    let el = document.elementFromPoint(x, y);
    if (!(el instanceof Element) || !b.contains(el)) el = b;
    return { el, x, y };
  }
  function pressTarget(t) {
    const o = { bubbles: true, cancelable: true, composed: true, view: window,
                clientX: t.x, clientY: t.y, button: 0,
                pointerId: 1, pointerType: 'mouse', isPrimary: true };
    for (const [C, type] of [
      [PointerEvent, 'pointerover'], [PointerEvent, 'pointerenter'],
      [PointerEvent, 'pointermove'], [PointerEvent, 'pointerdown'],
      [MouseEvent, 'mousedown'], [PointerEvent, 'pointerup'],
      [MouseEvent, 'mouseup'], [MouseEvent, 'click'],
    ]) {
      try { t.el.dispatchEvent(new C(type, o)); } catch (_) {}
    }
  }
  // Instagram mobile web's comment sheet has no close control at all:
  // it is a full-viewport role=presentation overlay dismissed by
  // tapping the backdrop above the sheet. Aim at the overlay's top
  // edge in that case; an explicit close button (YouTube, TikTok)
  // still wins when present.
  function commentCloseTarget() {
    const b = commentCloseButton();
    if (b) return buttonTarget(b);
    for (const p of document.querySelectorAll('[role="presentation"]')) {
      const r = p.getBoundingClientRect();
      if (r.width < innerWidth * 0.95 || r.height < innerHeight * 0.95) continue;
      const x = innerWidth / 2, y = 30;
      const el = document.elementFromPoint(x, y);
      if (el instanceof Element && p.contains(el)) return { el, x, y };
    }
    return null;
  }
  // ArrowLeft toggles: close the open sheet first, otherwise open it.
  // Without the close arm, pressing ArrowLeft twice only opens the sheet.
  function commentToggleTarget() {
    return commentCloseTarget() || buttonTarget(commentButton());
  }

  // Capture phase, ahead of the sites' own handlers: the point of the
  // unified scheme is that these keys behave identically on every
  // shortform feed, overriding per-site bindings (YouTube's seek on
  // ArrowRight, for example). Still guarded by ownsKeys so text entry
  // and native controls keep their keys, and still fail-open: a
  // shortcut only consumes the key when its control was found.
  addEventListener('keydown', e => {
    try {
      if (!shortformSite() || e.defaultPrevented || e.ctrlKey || e.altKey || e.metaKey
          || ownsKeys(e.target)) return;
      if (e.key === 'ArrowRight') {
        if (holdShortformSpeed()) { e.preventDefault(); e.stopPropagation(); }
      } else if (e.key === 'ArrowLeft' && !e.repeat) {
        const t = commentToggleTarget();
        if (t) { pressTarget(t); e.preventDefault(); e.stopPropagation(); }
      } else if (e.key === 'ArrowLeft' && e.repeat) {
        // A held left key must not click the toggle repeatedly. Still
        // consume it so the generic horizontal scroller cannot run.
        if (commentToggleTarget()) { e.preventDefault(); e.stopPropagation(); }
      } else if (e.key === ' ' && !e.repeat && !e.shiftKey) {
        if (toggleShortformPlayback()) { e.preventDefault(); e.stopPropagation(); }
      }
    } catch (_) {
      // Fail open: native/page keyboard handling remains available.
    }
  }, { capture: true, passive: false });
  addEventListener('keyup', e => {
    try {
      if (e.key === 'ArrowRight' && speedRestore.size) {
        releaseShortformSpeed();
        e.preventDefault();
        e.stopPropagation();
      }
    } catch (_) {
      releaseShortformSpeed();
    }
  }, { capture: true, passive: false });
  addEventListener('blur', releaseShortformSpeed);
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState !== 'visible') releaseShortformSpeed();
  });

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
      if (!el) {
        // IG's transform-virtualized feed can be unscrollable by
        // extent (see gestureFeed) while still swipeable.
        const feed = !k.h && gestureFeed(e.target);
        if (feed && swipeFeed(feed, k.dir)) e.preventDefault();
        return; // nothing scrollable: stay native
      }

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
        // Every preventDefault must follow its deciding guard. The
        // precise-touchpad arm decides via preciseFeedScroll; the
        // generic path decides via isDiscrete + scrollTarget.
        let handler = SMOOTH_WHEEL_JS
            .split("addEventListener('wheel'")
            .nth(1)
            .unwrap();
        let discrete = handler.find("isDiscrete").unwrap();
        let precise = handler.find("preciseFeedScroll").unwrap();
        let precise_pd = precise + handler[precise..].find("e.preventDefault()").unwrap();
        assert!(discrete < precise && precise < precise_pd);
        let target = handler.find("scrollTarget").unwrap();
        let pd = target + handler[target..].find("e.preventDefault()").unwrap();
        assert!(target < pd);
    }

    /// ArrowLeft must select the close control before falling back to the
    /// comment opener, otherwise pressing it twice only opens the sheet.
    #[test]
    fn comment_toggle_closes_open_sheet() {
        assert!(SMOOTH_WHEEL_JS.contains("function commentCloseButton"));
        assert!(SMOOTH_WHEEL_JS.contains("[role=\"dialog\"],[aria-modal=\"true\"]"));
        assert!(SMOOTH_WHEEL_JS.contains("[aria-label*=\"close\" i]"));
        let close = SMOOTH_WHEEL_JS.find("function commentCloseTarget").unwrap();
        let opener = SMOOTH_WHEEL_JS.find("function commentButton").unwrap();
        let toggle = SMOOTH_WHEEL_JS
            .find("return commentCloseTarget() || buttonTarget(commentButton());")
            .unwrap();
        assert!(close < toggle && opener < toggle);
        // Instagram's sheet has no close control: the backdrop
        // (full-viewport role=presentation overlay) is the dismissal
        // surface, so the close arm must know about it.
        assert!(SMOOTH_WHEEL_JS.contains("[role=\"presentation\"]"));
        // A held (repeating) ArrowLeft must not click the toggle again,
        // but still consume the key so the scroller cannot run.
        assert!(SMOOTH_WHEEL_JS.contains("e.key === 'ArrowLeft' && !e.repeat"));
        assert!(SMOOTH_WHEEL_JS
            .contains("if (commentToggleTarget()) { e.preventDefault(); e.stopPropagation(); }"));
    }

    /// Controls must be pressed with the full pointer-event contract:
    /// React (Instagram) binds to pointer events and ignores a bare
    /// el.click(), which made ArrowLeft a silent no-op on Reels.
    #[test]
    fn comment_toggle_uses_pointer_press() {
        assert!(SMOOTH_WHEEL_JS.contains("function pressTarget"));
        for ev in [
            "'pointerover'",
            "'pointerenter'",
            "'pointermove'",
            "'pointerdown'",
            "'mousedown'",
            "'pointerup'",
            "'mouseup'",
            "'click'",
        ] {
            let press = SMOOTH_WHEEL_JS
                .split("function pressTarget")
                .nth(1)
                .unwrap();
            let body = press.split("function commentCloseTarget").next().unwrap();
            assert!(body.contains(ev), "pressTarget missing {ev}");
        }
        // The press aims at the hit-test point inside the control, not
        // the wrapper element (React handlers live on inner nodes).
        assert!(SMOOTH_WHEEL_JS.contains("document.elementFromPoint"));
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
        // The IG gestureFeed fallback consumes first, but its decision
        // (swipeFeed) precedes its preventDefault; the generic glide's
        // preventDefault must still come after pageSnap.
        let fb = handler.find("gestureFeed(e.target)").unwrap();
        let fb_pd = fb + handler[fb..].find("e.preventDefault()").unwrap();
        let fb_swipe = fb + handler[fb..].find("swipeFeed").unwrap();
        assert!(
            fb_swipe < fb_pd,
            "fallback must decide before preventDefault"
        );
        let snap = handler.find("pageSnap").unwrap();
        let pd = snap + handler[snap..].find("e.preventDefault()").unwrap();
        assert!(snap < pd, "pageSnap must run before preventDefault");
    }

    /// The keyboard path must guard editable/interactive targets and
    /// defaultPrevented before any preventDefault, and must page snap
    /// containers before gliding — same fail-open contract as wheel.
    #[test]
    fn key_handler_shape() {
        let handler = SMOOTH_WHEEL_JS
            .split("addEventListener('keydown'")
            .nth(2)
            .expect("keydown handler present");
        // First preventDefault is the IG gestureFeed fallback; the
        // guards must precede even that one.
        let pd = handler.find("e.preventDefault()").unwrap();
        let owns = handler.find("ownsKeys").unwrap();
        let prevented = handler.find("e.defaultPrevented").unwrap();
        let snap = handler.find("pageSnap").unwrap();
        assert!(owns < pd, "ownsKeys guard must precede preventDefault");
        assert!(
            prevented < pd,
            "defaultPrevented check must precede preventDefault"
        );
        let glide_pd = snap + handler[snap..].find("e.preventDefault()").unwrap();
        assert!(
            snap < glide_pd,
            "pageSnap must run before glide preventDefault"
        );
        // Bubble phase, not capture: page handlers keep priority.
        assert!(!handler.starts_with(", e => {}, { capture: true"));
        for key in ["ArrowDown", "ArrowUp", "PageDown", "PageUp"] {
            assert!(SMOOTH_WHEEL_JS.contains(key), "{key} handled");
        }
    }

    /// The unified shortform shortcuts must be site/path-scoped (IG
    /// Reels, YT Shorts, TikTok), hold-sensitive for playback speed,
    /// toggle playback with Space, and use accessible controls rather
    /// than brittle generated class names.
    #[test]
    fn shortform_shortcuts_shape() {
        assert!(SMOOTH_WHEEL_JS.contains("function shortformSite"));
        assert!(SMOOTH_WHEEL_JS.contains("instagram.com"));
        assert!(SMOOTH_WHEEL_JS.contains("youtube.com"));
        assert!(SMOOTH_WHEEL_JS.contains("tiktok.com"));
        assert!(SMOOTH_WHEEL_JS.contains("speedRestore"));
        assert!(SMOOTH_WHEEL_JS.contains("v.playbackRate = 2"));
        assert!(SMOOTH_WHEEL_JS.contains("v.playbackRate = rate"));
        assert!(SMOOTH_WHEEL_JS.contains("svg[aria-label=\"Comment\" i]"));
        assert!(SMOOTH_WHEEL_JS.contains("pressTarget(t)"));
        assert!(SMOOTH_WHEEL_JS.contains("toggleShortformPlayback"));
        assert!(SMOOTH_WHEEL_JS.contains("v.pause()"));
        assert!(SMOOTH_WHEEL_JS.contains("document.addEventListener('visibilitychange'"));

        let shortcut = SMOOTH_WHEEL_JS
            .split("function shortformSite")
            .nth(1)
            .and_then(|tail| tail.split("// Targets that own their keys").next())
            .expect("shortform shortcut block present");
        assert!(shortcut.contains("ArrowRight"));
        assert!(shortcut.contains("ArrowLeft"));
        assert!(shortcut.contains("e.key === ' '"));
        assert!(shortcut.contains("e.repeat"));
        // The shortcut listeners run in capture phase so per-site key
        // bindings (YouTube seek, TikTok volume) cannot shadow the
        // unified scheme; ownsKeys still protects text entry.
        assert!(shortcut.contains("{ capture: true, passive: false }"));
        assert!(shortcut.contains("ownsKeys(e.target)"));
    }

    /// Snapless card feeds (Reels mobile web) must be caught by the
    /// uniform-card heuristic, and the heuristic must require uniform
    /// viewport-sized children so ordinary documents never page.
    #[test]
    fn card_feed_heuristic_shape() {
        assert!(SMOOTH_WHEEL_JS.contains("feedOffsets"));
        // Uniformity gate present (>= 80% of measured children).
        assert!(SMOOTH_WHEEL_JS.contains("uniform / measured >= 0.8"));
        // Fallback order inside pageSnap: CSS snap first, then feed
        // (the IG gesture gate also calls feedOffsets earlier, so
        // compare against the fallback occurrence specifically).
        let ps = SMOOTH_WHEEL_JS.split("function pageSnap").nth(1).unwrap();
        let css = ps.find("isMandatorySnap").unwrap();
        let feed = ps[css..].find("feedOffsets").map(|i| css + i).unwrap();
        assert!(css < feed, "CSS snap must be preferred over the heuristic");
    }

    /// Instagram's reel feed advances only from touch gestures:
    /// scrollTop paging freezes its URL/pagination state. Paging there
    /// must dispatch a synthetic pointer swipe, gated to the IG
    /// shortform URL + uniform-card feed so nothing else is touched,
    /// and the setPointerCapture guard must rethrow for real pointers.
    #[test]
    fn instagram_swipe_paging_shape() {
        assert!(SMOOTH_WHEEL_JS.contains("function swipeFeed"));
        // Gate: vertical + instagram + card feed, checked inside pageSnap
        // before any scrollTop-based paging.
        let ps = SMOOTH_WHEEL_JS.split("function pageSnap").nth(1).unwrap();
        let gate = ps
            .find("shortformSite() === 'instagram'")
            .expect("IG gesture gate present in pageSnap");
        let css = ps.find("isMandatorySnap").unwrap();
        assert!(gate < css, "gesture gate must precede scrollTop paging");
        // The feed may report no scrollable extent at all (transform
        // virtualization): both input paths must fall back to the
        // gestureFeed locator instead of staying native.
        assert!(SMOOTH_WHEEL_JS.contains("function gestureFeed"));
        assert_eq!(SMOOTH_WHEEL_JS.matches("gestureFeed(e.target)").count(), 2);
        // The gesture is a touch-typed primary pointer with a stable
        // synthetic id, and repeated ticks are absorbed during flight.
        assert!(SMOOTH_WHEEL_JS.contains("pointerType: 'touch'"));
        assert!(SMOOTH_WHEEL_JS.contains("pointerId: SWIPE_ID"));
        assert!(SMOOTH_WHEEL_JS.contains("now < swipeUntil"));
        // Pointer-capture guard: swallow only our synthetic id.
        assert!(SMOOTH_WHEEL_JS.contains("if (id !== SWIPE_ID) throw err;"));
    }

    /// Paging a shortform feed must perform the commit-time playback
    /// handoff (incoming video plays at gesture commit; outgoing audio
    /// ramps out then pauses), and must not crossfade: the pause and
    /// a volume restore both happen, and everything is guarded so a
    /// missing video fails open to the site's own observer.
    #[test]
    fn commit_time_playback_handoff_shape() {
        assert!(SMOOTH_WHEEL_JS.contains("function handoffPlayback"));
        assert!(SMOOTH_WHEEL_JS.contains("function rampVolume"));
        // Gated to shortform sites: nothing happens on ordinary pages.
        let body = SMOOTH_WHEEL_JS
            .split("function handoffPlayback")
            .nth(1)
            .unwrap();
        assert!(body.contains("if (!shortformSite()) return;"));
        // Outgoing: ramp to zero, pause, then restore the element's
        // own volume so scroll-back never inherits a silenced video.
        assert!(body.contains("rampVolume(out, 0, RAMP_OUT_MS"));
        assert!(body.contains("out.pause(); out.volume = ov;"));
        // Incoming: starts muted-by-volume and ramps in; the play()
        // rejection is swallowed (autoplay policy).
        assert!(body.contains("inc.volume = 0;"));
        assert!(body.contains("p.catch(() => {})"));
        // Both commit paths fire it: the synthetic IG swipe at
        // pointerup, and snap/card-feed paging at animation start.
        let swipe = SMOOTH_WHEEL_JS.split("function swipeFeed").nth(1).unwrap();
        let sw_body = swipe.split("function gestureFeed").next().unwrap();
        assert!(sw_body.contains("handoffPlayback(dir)"));
        let ps = SMOOTH_WHEEL_JS.split("function pageSnap").nth(1).unwrap();
        let ps_body = ps.split("function animateBy").next().unwrap();
        assert!(ps_body.contains("handoffPlayback(dir)"));
    }

    /// Precise touchpad deltas on the Instagram gesture feed must be
    /// claimed (native precise scroll desyncs IG's gesture-held reel
    /// state) and accumulated into the same synthetic swipe; precise
    /// deltas everywhere else must stay engine-native.
    #[test]
    fn precise_touchpad_guard_shape() {
        assert!(SMOOTH_WHEEL_JS.contains("function preciseFeedScroll"));
        let body = SMOOTH_WHEEL_JS
            .split("function preciseFeedScroll")
            .nth(1)
            .unwrap();
        // Instagram-only: everything else returns false -> native.
        assert!(body.contains("shortformSite() !== 'instagram'"));
        // Pages via the synthetic swipe, never scrollTop.
        assert!(body.contains("swipeFeed(feed, dir)"));
        // Mid-swipe flicks are absorbed like discrete ticks.
        assert!(body.contains("now < swipeUntil"));
        // The wheel handler consults it inside the !isDiscrete arm and
        // preventDefaults claimed events so no pixels leak to the feed.
        let handler = SMOOTH_WHEEL_JS
            .split("addEventListener('wheel'")
            .nth(1)
            .unwrap();
        let arm = handler.split("let dx = e.deltaX").next().unwrap();
        assert!(arm.contains("!isDiscrete(e)"));
        assert!(arm.contains("preciseFeedScroll(e.target, e.deltaY)) e.preventDefault()"));
    }
}
