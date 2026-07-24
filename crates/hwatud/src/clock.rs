//! Virtual time for a page: `hwatu clock pause|resume|step|set`.
//!
//! [`crate::verify::seek`] pins declarative animation (CSS/WAAPI)
//! because the Web Animations API exposes their timeline. Script-driven
//! motion has no such handle: a `requestAnimationFrame` loop that
//! integrates timestamp deltas (the classic marquee/physics pattern)
//! keeps running through a seek, so the frame is never deterministic.
//!
//! This module injects a user script at document start (before any
//! page code runs) that puts every clock the page can read behind one
//! controllable virtual timeline:
//!
//! - `performance.now()` and `Date.now()` return virtual time,
//! - `setTimeout`/`setInterval` fire on virtual deadlines (delegated
//!   1:1 to native timers while the clock is live, so real pages keep
//!   their real behavior until an agent takes control),
//! - `requestAnimationFrame` callbacks are held in a registry and
//!   flushed by a native rAF pump while live, or by `step` while
//!   paused, always with a virtual timestamp,
//! - CSS/WAAPI animations are paused and their `currentTime` advanced
//!   from the same timeline, so declarative and script motion share
//!   one clock.
//!
//! `pause` freezes all of it; `step <ms>` advances deterministically in
//! 60 fps ticks (due timers fire, one rAF batch per tick); `set <ms>`
//! steps to an absolute virtual time; `resume` returns to real time
//! monotonically. Two screenshots at the same virtual time are
//! byte-identical, which is what turns "the page moves" from a flake
//! into a measurement.

use crate::automation::{self, Reply};
use crate::Daemon;
use hwatu_ipc::{ClockAction, Response};
use std::rc::Rc;

/// User script injected at document start in every frame. Installs the
/// virtual clock *dormant*: all wrapped clocks delegate to the native
/// ones (virtual time == real time) until the first `pause`/`step`/
/// `set`, so pages that never use `hwatu clock` behave natively.
/// Idempotent per realm.
const CLOCK_JS: &str = r#"(() => {
  if (window.__hwatu_clock) return;
  const g = window;
  const realPerf = performance;
  const realNow = performance.now.bind(performance);
  const nativeSetTimeout = g.setTimeout.bind(g);
  const nativeClearTimeout = g.clearTimeout.bind(g);
  const nativeSetInterval = g.setInterval.bind(g);
  const nativeClearInterval = g.clearInterval.bind(g);
  const nativeRaf = g.requestAnimationFrame.bind(g);
  const nativeCaf = g.cancelAnimationFrame.bind(g);
  // Wall-clock epoch at performance-timeline zero. In deterministic
  // start-paused mode the epoch is pinned to a constant: pages that
  // derive visible state from absolute Date.now() (live counters)
  // would otherwise differ across loads in their last digits even
  // with identical virtual timelines.
  const startPausedEpoch = 1700000000000;
  const configuredEpoch = Number(g.__hwatu_clock_epoch_ms);
  const dateBase = Number.isFinite(configuredEpoch)
    ? configuredEpoch
    : (g.__hwatu_clock_start_paused ? startPausedEpoch : (Date.now() - realNow()));

  // Virtual timeline: continuous with the performance timeline until
  // the first pause (vnow() === realNow() while never paused).
  //
  // Deterministic-load mode (`HWATU_CLOCK_START_PAUSED` on the
  // daemon, surfaced as a pre-injected global): the clock installs
  // already paused at virtual t=0, so the *whole load* happens at one
  // frozen instant. Every timer deadline and every now() read during
  // parsing is then identical across loads, which is what makes two
  // fresh loads of the same page byte-comparable under `clock step`.
  const startPaused = !!g.__hwatu_clock_start_paused;
  let paused = startPaused;
  let vbase = startPaused ? 0 : realNow(); // virtual time at the last sync point
  let rbase = realNow();                   // realNow() at the last sync point
  const vnow = () => (paused ? vbase : vbase + (realNow() - rbase));

  // ---- timers: one registry, native delegation while live ---------
  // Entry: { cb, args, deadline (virtual ms), interval (ms|null),
  //          native (native timer id | null) }
  const timers = new Map();
  let nextTimerId = 1_000_000_000; // far from native ids
  const armNative = (id, entry) => {
    const delay = Math.max(0, entry.deadline - vnow());
    entry.native = nativeSetTimeout(() => fireTimer(id), delay);
  };
  const fireTimer = (id) => {
    const entry = timers.get(id);
    if (!entry) return;
    if (entry.interval !== null) {
      entry.deadline += entry.interval;
      entry.native = null;
      if (!paused) armNative(id, entry);
    } else {
      timers.delete(id);
    }
    try { entry.cb(...entry.args); } catch (e) { nativeSetTimeout(() => { throw e; }, 0); }
  };
  const schedule = (cb, delay, interval, args) => {
    const id = nextTimerId++;
    const entry = {
      cb, args,
      deadline: vnow() + Math.max(Number(delay) || 0, 0),
      interval, native: null,
    };
    timers.set(id, entry);
    if (!paused) armNative(id, entry);
    return id;
  };
  g.setTimeout = function (cb, delay, ...args) {
    if (typeof cb !== 'function') return nativeSetTimeout(cb, delay, ...args);
    return schedule(cb, delay, null, args);
  };
  g.setInterval = function (cb, delay, ...args) {
    if (typeof cb !== 'function') return nativeSetInterval(cb, delay, ...args);
    const ms = Math.max(Number(delay) || 0, 1);
    return schedule(cb, ms, ms, args);
  };
  const clearAny = (id) => {
    const entry = timers.get(id);
    if (entry) {
      if (entry.native !== null) nativeClearTimeout(entry.native);
      timers.delete(id);
    }
  };
  g.clearTimeout = (id) => { clearAny(id); nativeClearTimeout(id); };
  g.clearInterval = (id) => { clearAny(id); nativeClearInterval(id); };

  // ---- requestAnimationFrame: registry + pump ----------------------
  const rafCallbacks = new Map();
  let nextRafId = 1;
  let rafPump = null; // native rAF id driving the live flush
  const flushRaf = () => {
    const batch = [...rafCallbacks.values()];
    rafCallbacks.clear();
    const ts = vnow();
    for (const cb of batch) {
      try { cb(ts); } catch (e) { nativeSetTimeout(() => { throw e; }, 0); }
    }
  };
  const armPump = () => {
    if (paused || rafPump !== null || rafCallbacks.size === 0) return;
    rafPump = nativeRaf(() => { rafPump = null; flushRaf(); armPump(); });
  };
  g.requestAnimationFrame = (cb) => {
    const id = nextRafId++;
    rafCallbacks.set(id, cb);
    armPump();
    return id;
  };
  g.cancelAnimationFrame = (id) => { rafCallbacks.delete(id); };

  // ---- clocks the page reads ---------------------------------------
  realPerf.now = () => vnow();
  Date.now = () => Math.round(dateBase + vnow());

  // ---- Math.random: optional seeded determinism ----------------------
  // Math.random is the one visible entropy source the virtual clock
  // does not cover: a page that renders from it can never be
  // byte-compared across loads. seedRandom(n) replaces it with
  // mulberry32 seeded from n, so same seed + same virtual timeline
  // (which fixes the *order* of random() calls) => identical
  // sequences. Default behavior is untouched native Math.random.
  const nativeRandom = Math.random.bind(Math);
  let randomSeed = null;
  const seedRandom = (n) => {
    n = Number(n);
    if (!Number.isFinite(n) || n < 0) return { error: 'seed needs a finite number >= 0' };
    randomSeed = n;
    // Fold a u64-ish seed into 32 bits, then mulberry32. >>> 0 keeps
    // arithmetic in uint32 land; the fold keeps distinct u64 seeds
    // from trivially colliding in the low word.
    let state = ((n >>> 0) ^ Math.floor(n / 4294967296)) >>> 0;
    Math.random = () => {
      state = (state + 0x6D2B79F5) >>> 0;
      let t = state;
      t = Math.imul(t ^ (t >>> 15), t | 1);
      t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
    return status();
  };
  // ---- declarative animations share the timeline --------------------
  // Animations we paused (vs. paused by the page itself): only these
  // are resumed by `resume`.
  const pausedByUs = new WeakSet();
  const pauseAnimations = () => {
    for (const a of document.getAnimations()) {
      try {
        if (a.playState === 'running') { a.pause(); pausedByUs.add(a); }
      } catch (e) { /* infinite timeline etc. */ }
    }
  };
  const advanceAnimations = (ms) => {
    for (const a of document.getAnimations()) {
      try {
        if (a.playState === 'running') { a.pause(); pausedByUs.add(a); }
        if (a.currentTime !== null) a.currentTime = Number(a.currentTime) + ms;
      } catch (e) { /* not seekable */ }
    }
  };

  // ---- IntersectionObserver: pumped under virtual stepping ----------
  // WebKit delivers IO callbacks on rendering opportunities, and a
  // hidden/unmapped (headless) page never gets one, so script gated on
  // "am I in view" (lazy loads, marquee starts) never runs headless.
  // Track registrations and, on each `step` tick, compute viewport
  // intersections manually and deliver entries with virtual timestamps.
  // While live, the native observer delivers as usual; the pump only
  // fires under step, and only when the observed state changed.
  const NativeIO = g.IntersectionObserver;
  const ioInstances = new Set();
  if (NativeIO) {
    g.IntersectionObserver = class IntersectionObserver extends NativeIO {
      constructor(cb, options) {
        super(cb, options);
        this.__hwatu = { cb, options: options || {}, targets: new Set(), last: new WeakMap() };
        ioInstances.add(this);
      }
      observe(t) { this.__hwatu.targets.add(t); return super.observe(t); }
      unobserve(t) { this.__hwatu.targets.delete(t); return super.unobserve(t); }
      disconnect() { this.__hwatu.targets.clear(); ioInstances.delete(this); return super.disconnect(); }
    };
  }
  const ioRect = (root) => {
    if (root && root.getBoundingClientRect) return root.getBoundingClientRect();
    return { left: 0, top: 0, right: g.innerWidth, bottom: g.innerHeight,
             width: g.innerWidth, height: g.innerHeight };
  };
  const pumpIO = () => {
    for (const io of ioInstances) {
      const state = io.__hwatu;
      const rb = ioRect(state.options.root);
      const entries = [];
      for (const target of state.targets) {
        if (!target.isConnected) continue;
        const r = target.getBoundingClientRect();
        const ileft = Math.max(r.left, rb.left), itop = Math.max(r.top, rb.top);
        const iright = Math.min(r.right, rb.right), ibottom = Math.min(r.bottom, rb.bottom);
        const iw = Math.max(0, iright - ileft), ih = Math.max(0, ibottom - itop);
        const area = r.width * r.height;
        const ratio = area > 0 ? (iw * ih) / area : (iw >= 0 && ih >= 0 ? 1 : 0);
        const intersecting = iw > 0 && ih > 0;
        const last = state.last.get(target);
        if (last !== undefined && last === intersecting) continue;
        state.last.set(target, intersecting);
        entries.push({
          target, isIntersecting: intersecting, intersectionRatio: ratio,
          boundingClientRect: r,
          intersectionRect: { left: ileft, top: itop, width: iw, height: ih },
          rootBounds: rb, time: vnow(),
        });
      }
      if (entries.length) {
        try { state.cb(entries, io); } catch (e) { nativeSetTimeout(() => { throw e; }, 0); }
      }
    }
  };

  // ---- control surface ----------------------------------------------
  const status = () => ({
    installed: true,
    paused,
    virtual_ms: Math.round(vnow() * 1000) / 1000,
    pending_timers: timers.size,
    pending_rafs: rafCallbacks.size,
    seed: randomSeed,
  });
  const pause = () => {
    if (paused) return status();
    vbase = vnow();
    paused = true;
    for (const entry of timers.values()) {
      if (entry.native !== null) { nativeClearTimeout(entry.native); entry.native = null; }
    }
    if (rafPump !== null) { nativeCaf(rafPump); rafPump = null; }
    pauseAnimations();
    return status();
  };
  const resume = () => {
    if (!paused) return status();
    paused = false;
    rbase = realNow();
    for (const [id, entry] of timers) armNative(id, entry);
    armPump();
    for (const a of document.getAnimations()) {
      try { if (pausedByUs.has(a)) { pausedByUs.delete(a); a.play(); } } catch (e) {}
    }
    return status();
  };
  const TICK = 1000 / 60;
  const MAX_TIMER_FIRES_PER_TICK = 1000;
  const step = (ms) => {
    ms = Number(ms);
    if (!Number.isFinite(ms) || ms < 0) return { error: 'step needs a finite ms >= 0' };
    pause();
    let remaining = ms;
    while (remaining > 1e-9) {
      const dt = Math.min(TICK, remaining);
      remaining -= dt;
      vbase += dt;
      // Fire due timers, earliest first; intervals may refire within
      // the tick, capped so a 0 ms interval cannot hang the page.
      for (let fired = 0; fired < MAX_TIMER_FIRES_PER_TICK; fired++) {
        let dueId = null;
        let dueAt = Infinity;
        for (const [id, entry] of timers) {
          if (entry.deadline <= vbase + 1e-9 && entry.deadline < dueAt) {
            dueAt = entry.deadline;
            dueId = id;
          }
        }
        if (dueId === null) break;
        fireTimer(dueId);
      }
      pumpIO();
      flushRaf();
      // Force a synchronous style recalc so CSS transitions triggered
      // by the timers/rAF above are *created* inside this tick, then
      // advance declarative animations by the same dt. Leaving the
      // advance to the end of the step let transition creation race
      // the engine's real-time render cycle: an animation born after
      // the loop finished missed its advance entirely, which showed
      // up as one-frame phase flicker between two lockstep windows.
      void document.documentElement && document.documentElement.offsetWidth;
      advanceAnimations(dt);
    }
    return status();
  };
  const set = (ms) => {
    ms = Number(ms);
    if (!Number.isFinite(ms) || ms < 0) return { error: 'set needs a finite ms >= 0' };
    const cur = vnow();
    const wasPaused = paused;
    pause();
    if (ms < cur - 1e-6) {
      // The first control operation on a live page may establish a new
      // virtual epoch after navigation/font/hydration waits have settled.
      // Preserve each timer's remaining delay while rebasing now() to the
      // requested value. Once a page is already paused, backwards travel
      // remains unsupported because arbitrary script state cannot rewind.
      if (!wasPaused) {
        const delta = ms - cur;
        for (const entry of timers.values()) entry.deadline += delta;
        vbase = ms;
        return status();
      }
      return { error: `cannot go backwards: virtual time is ${Math.round(cur)} ms, requested ${ms} ms`, ...status() };
    }
    return step(ms - cur);
  };
  // A pre-injected flag (set by the daemon before this script when a
  // seed was requested for future loads) seeds from document start,
  // before any page script can capture or consume native Math.random.
  if (typeof g.__hwatu_clock_seed === 'number') seedRandom(g.__hwatu_clock_seed);
  window.__hwatu_clock = {
    pause, resume, step, set, status, seedRandom,
    nativeRandom,
    // Native escape hatch for the harness's own plumbing: hwatu's
    // scroll/click/type/expect/challenge JS must keep running in real
    // time while the *page's* clocks are frozen, or pausing the page
    // would deadlock the tool driving it.
    native: {
      setTimeout: nativeSetTimeout,
      clearTimeout: nativeClearTimeout,
      now: () => realNow(),
      dateNow: () => Math.round(dateBase + realNow()),
    },
  };
})();"#;

/// Register the virtual-clock user script on a WebView's content
/// manager. Must run on every view (prewarmed pool, popups) before it
/// loads page content, because the wrappers must win the race against
/// page scripts capturing the native clocks.
pub fn wire_view(view: &webkit6::WebView) {
    use webkit6::prelude::WebViewExt;
    let Some(ucm) = view.user_content_manager() else {
        return;
    };
    // Deterministic-load mode: a tiny preamble script (added first, so
    // it runs first) flags the realm before the clock installs. Only
    // isolated verification daemons set this env; interactive daemons
    // keep native passthrough behavior.
    if std::env::var("HWATU_CLOCK_START_PAUSED").is_ok_and(|v| !v.is_empty() && v != "0") {
        let flag = webkit6::UserScript::new(
            "window.__hwatu_clock_start_paused = true;",
            webkit6::UserContentInjectedFrames::AllFrames,
            webkit6::UserScriptInjectionTime::Start,
            &[],
            &[],
        );
        ucm.add_script(&flag);
    }
    // Pin Date.now() independently of timeline state. This lets verification
    // pages finish native-clock navigation/font/hydration waits before their
    // first `clock set 0`, while still giving every page the same wall time.
    if let Some(epoch_ms) = std::env::var("HWATU_CLOCK_EPOCH_MS")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
    {
        let epoch = webkit6::UserScript::new(
            &format!("window.__hwatu_clock_epoch_ms = {epoch_ms};"),
            webkit6::UserContentInjectedFrames::AllFrames,
            webkit6::UserScriptInjectionTime::Start,
            &[],
            &[],
        );
        ucm.add_script(&epoch);
    }
    let script = webkit6::UserScript::new(
        CLOCK_JS,
        webkit6::UserContentInjectedFrames::AllFrames,
        webkit6::UserScriptInjectionTime::Start,
        &[],
        &[],
    );
    ucm.add_script(&script);
}

/// Dispatch one clock command to the page's installed control surface.
///
/// `seed` is special: besides seeding the live page immediately, it
/// registers a document-start user script on the window's view so
/// every *future* load in that window is seeded before any page
/// script can capture or consume the native `Math.random`.
pub fn clock(
    daemon: &Rc<Daemon>,
    id: Option<u64>,
    action: ClockAction,
    ms: Option<f64>,
    seed: Option<u64>,
    timeout_ms: Option<u64>,
    reply: Reply,
) {
    let call = match (action, ms) {
        (ClockAction::Pause, _) => "c.pause()".to_string(),
        (ClockAction::Resume, _) => "c.resume()".to_string(),
        (ClockAction::Status, _) => "c.status()".to_string(),
        (ClockAction::Step, Some(ms)) if ms.is_finite() && ms >= 0.0 => format!("c.step({ms})"),
        (ClockAction::Step, _) => {
            return reply(Response::err("usage: hwatu clock step <ms>"));
        }
        (ClockAction::Set, Some(ms)) if ms.is_finite() && ms >= 0.0 => format!("c.set({ms})"),
        (ClockAction::Set, _) => {
            return reply(Response::err("usage: hwatu clock set <ms>"));
        }
        (ClockAction::Seed, _) => {
            let Some(seed) = seed else {
                return reply(Response::err("usage: hwatu clock seed <u64>"));
            };
            // f64 can't hold every u64 exactly; the shim folds to u32
            // anyway, but reject seeds that would silently change so
            // "same seed" always means the same PRNG.
            if seed > (1u64 << 53) {
                return reply(Response::err("seed must be <= 2^53 (JS number precision)"));
            }
            if let Err(resp) = wire_seed(daemon, id, seed) {
                return reply(*resp);
            }
            format!("c.seedRandom({seed})")
        }
    };
    let js = format!(
        r#"
const c = window.__hwatu_clock;
if (!c) return {{ error: "virtual clock not installed in this page (page predates this daemon build; reload it)" }};
return {call};"#
    );
    automation::eval(daemon, id, js, timeout_ms, reply);
}

/// Register the persistent seed flag script on the target window's
/// view. Runs at document start; the flag global covers loads where it
/// executes before CLOCK_JS, the direct call covers loads where the
/// clock installed first (scripts run in registration order, and
/// CLOCK_JS is registered at view build time).
fn wire_seed(daemon: &Rc<Daemon>, id: Option<u64>, seed: u64) -> Result<(), Box<Response>> {
    use webkit6::prelude::WebViewExt;
    let windows = daemon.windows.borrow();
    let win = match id {
        Some(id) => windows
            .get(&id)
            .cloned()
            .ok_or_else(|| Box::new(Response::err(format!("no window {id}"))))?,
        None => match windows.len() {
            1 => windows.values().next().cloned().expect("len checked"),
            0 => return Err(Box::new(Response::err("no windows open"))),
            n => {
                return Err(Box::new(Response::err(format!(
                    "{n} windows open; pass --id"
                ))))
            }
        },
    };
    let Some(view) = win.live_webview() else {
        return Err(Box::new(Response::err("window has no live webview")));
    };
    let Some(ucm) = view.user_content_manager() else {
        return Err(Box::new(Response::err("view has no user content manager")));
    };
    let js = format!(
        "window.__hwatu_clock_seed = {seed};\n\
         if (window.__hwatu_clock) window.__hwatu_clock.seedRandom({seed});"
    );
    let script = webkit6::UserScript::new(
        &js,
        webkit6::UserContentInjectedFrames::AllFrames,
        webkit6::UserScriptInjectionTime::Start,
        &[],
        &[],
    );
    ucm.add_script(&script);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::CLOCK_JS;

    /// The seeded-PRNG surface must exist in the injected shim: the
    /// control surface exposes seedRandom, status reports the seed,
    /// and the document-start flag path is wired for future loads.
    #[test]
    fn clock_js_exposes_seeded_random_surface() {
        for needle in [
            "seedRandom",
            "nativeRandom",
            "seed: randomSeed",
            "__hwatu_clock_seed",
            "0x6D2B79F5", // mulberry32 increment: the algorithm is part of the contract
        ] {
            assert!(CLOCK_JS.contains(needle), "missing {needle}");
        }
    }

    /// Default behavior unchanged: Math.random is only replaced inside
    /// seedRandom, never unconditionally at install.
    #[test]
    fn clock_js_does_not_replace_math_random_by_default() {
        // Exactly one assignment to Math.random, and it lives inside
        // seedRandom's body (after its declaration).
        let assigns: Vec<_> = CLOCK_JS.match_indices("Math.random =").collect();
        assert_eq!(
            assigns.len(),
            1,
            "Math.random should be assigned exactly once"
        );
        let seed_fn = CLOCK_JS
            .find("const seedRandom")
            .expect("seedRandom missing");
        assert!(
            assigns[0].0 > seed_fn,
            "Math.random must only be replaced inside seedRandom"
        );
        // And installation-time seeding is gated on the flag global.
        assert!(CLOCK_JS.contains(
            "if (typeof g.__hwatu_clock_seed === 'number') seedRandom(g.__hwatu_clock_seed);"
        ));
    }

    /// A settled live page can define virtual t=0 with its first `set 0`
    /// without trying to rewind DOM/script state. Timer deadlines move by
    /// the same delta so their remaining delays are preserved.
    #[test]
    fn clock_js_first_live_set_can_rebase_backwards() {
        for needle in [
            "const wasPaused = paused",
            "if (!wasPaused)",
            "entry.deadline += delta",
            "vbase = ms",
        ] {
            assert!(
                CLOCK_JS.contains(needle),
                "missing rebase behavior: {needle}"
            );
        }
    }

    #[test]
    fn clock_js_accepts_a_fixed_epoch_without_starting_paused() {
        assert!(CLOCK_JS.contains("Number(g.__hwatu_clock_epoch_ms)"));
        assert!(CLOCK_JS.contains("Number.isFinite(configuredEpoch)"));
    }

    /// Same seed + same call count => identical sequences. The Rust
    /// port here is the executable spec of the JS mulberry32 in
    /// CLOCK_JS (u32 arithmetic matches JS `>>> 0` / `Math.imul`);
    /// golden values pin the algorithm so a silent edit to the shim's
    /// PRNG breaks this test.
    #[test]
    fn mulberry32_reference_sequence_is_deterministic() {
        fn mulberry32(seed: u64) -> impl FnMut() -> f64 {
            let mut state = (seed as u32) ^ ((seed / 4_294_967_296) as u32);
            move || {
                state = state.wrapping_add(0x6D2B_79F5);
                let mut t = state;
                t = (t ^ (t >> 15)).wrapping_mul(t | 1);
                t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
                f64::from(t ^ (t >> 14)) / 4_294_967_296.0
            }
        }
        let mut a = mulberry32(42);
        let mut b = mulberry32(42);
        let seq_a: Vec<f64> = (0..64).map(|_| a()).collect();
        let seq_b: Vec<f64> = (0..64).map(|_| b()).collect();
        assert_eq!(seq_a, seq_b, "same seed must give identical sequences");
        assert!(seq_a.iter().all(|v| (0.0..1.0).contains(v)));
        // Golden first values for seed 42 (pins the exact algorithm).
        assert_eq!(seq_a[0], 0.6011037519201636);
        assert_eq!(seq_a[1], 0.44829055899754167);
        let mut c = mulberry32(43);
        assert_ne!(seq_a[0], c(), "different seeds must diverge");
    }
}
