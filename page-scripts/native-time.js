// hwatuSleep/hwatuNow on the native clock, so a paused virtual clock
// (`hwatu clock pause`) cannot stall automation waits.
const __hwatuNative = (window.__hwatu_clock && window.__hwatu_clock.native) || null;
const hwatuSleep = (ms) => new Promise((r) =>
  __hwatuNative ? __hwatuNative.setTimeout(r, ms) : setTimeout(r, ms));
const hwatuNow = () => (__hwatuNative ? __hwatuNative.dateNow() : Date.now());
// After a form submit, wait for the navigation the submit queued, so the
// reply reports it instead of answering before WebKit starts it (Linux: 1
// in 60 `type --enter` replies came first, and a following `wait-load`
// returned on the old page). Once the page starts unloading, keep waiting:
// the commit destroys this context and the daemon replies `navigated`
// after the load. If nothing unloads within `ms` (a JS-handled submit),
// return. A navigation that never commits (204, cancelled) returns after
// 10 s.
const hwatuAwaitUnload = (ms) => new Promise((r) => {
  const later = __hwatuNative ? __hwatuNative.setTimeout : setTimeout;
  let unloading = false;
  const start = () => { unloading = true; later(r, 10000); };
  window.addEventListener('beforeunload', start, { once: true });
  window.addEventListener('pagehide', start, { once: true });
  later(() => { if (!unloading) r(); }, ms);
});
