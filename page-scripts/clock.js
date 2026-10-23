(() => {
  if (window.__hwatu_clock) return;
  const g = window;
  const realPerf = performance;
  const realNow = performance.now.bind(performance);
  const NativeDate = g.Date;
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
    : (g.__hwatu_clock_start_paused ? startPausedEpoch : (NativeDate.now() - realNow()));

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
  const virtualDateNow = () => Math.round(dateBase + vnow());
  // Date.now alone is insufficient: sites commonly choose visible
  // day/night variants with `new Date().getHours()`. A Proxy preserves
  // Date's native prototype, parse/UTC statics, subclass construction,
  // and explicit-argument parsing while virtualizing only the implicit
  // "now" paths (`Date()` and `new Date()`).
  const VirtualDate = new Proxy(NativeDate, {
    apply() {
      return new NativeDate(virtualDateNow()).toString();
    },
    construct(target, args, newTarget) {
      return Reflect.construct(
        target,
        args.length === 0 ? [virtualDateNow()] : args,
        newTarget,
      );
    },
    get(target, property, receiver) {
      if (property === 'now') return virtualDateNow;
      return Reflect.get(target, property, receiver);
    },
  });
  g.Date = VirtualDate;

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
        const state = { cb, options: options || {}, targets: new Set(), last: new WeakMap() };
        super((entries, observer) => {
          // Keep the manual headless pump in sync with native delivery so a
          // rendering opportunity immediately before a pump cannot invoke the
          // page callback twice for the same state.
          for (const entry of entries) state.last.set(entry.target, entry.isIntersecting);
          cb(entries, observer);
        }, options);
        this.__hwatu = state;
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

  // ---- scroll events: pumped under virtual stepping ------------------
  // WebKit dispatches `scroll` events on rendering opportunities, so a
  // headless/unmapped page updates window.scrollY synchronously on
  // scrollTo() but never fires the event. Scroll-driven libraries
  // (Lenis, GSAP ScrollTrigger) hang off that event and never notice
  // the move. On each `step` tick, compare the window scroll offset
  // against the last delivered value and dispatch a plain `scroll`
  // event on document (bubbling to window) when it changed. Element
  // scrollers are not tracked: agent scrolls drive the window.
  let lastScrollX = g.scrollX;
  let lastScrollY = g.scrollY;
  const syncScroll = () => {
    lastScrollX = g.scrollX;
    lastScrollY = g.scrollY;
  };
  // Native scroll delivery, when WebKit does get a rendering opportunity,
  // wins over the fallback pump and suppresses a duplicate synthetic event.
  document.addEventListener('scroll', syncScroll, { capture: true });
  const pumpScroll = () => {
    if (g.scrollX === lastScrollX && g.scrollY === lastScrollY) return;
    lastScrollX = g.scrollX;
    lastScrollY = g.scrollY;
    try {
      document.dispatchEvent(new Event('scroll', { bubbles: true }));
    } catch (e) {}
  };
  const pumpViewport = () => {
    pumpScroll();
    pumpIO();
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
      pumpScroll();
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
      pumpViewport,
    },
  };
})();
