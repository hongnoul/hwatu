// hwatuSleep/hwatuNow on the native clock, so a paused virtual clock
// (`hwatu clock pause`) cannot stall automation waits.
const __hwatuNative = (window.__hwatu_clock && window.__hwatu_clock.native) || null;
const hwatuSleep = (ms) => new Promise((r) =>
  __hwatuNative ? __hwatuNative.setTimeout(r, ms) : setTimeout(r, ms));
const hwatuNow = () => (__hwatuNative ? __hwatuNative.dateNow() : Date.now());
