# Reducing input lag in the hwatu human UX

Status: research, 2026-07-30. Code references are to `main` at the
time of writing. Companion to the "adequate for a human minute"
stance in [roadmap.md](roadmap.md): the human path only needs to be
*responsive*, not featureful, and today several structural sources of
input latency work against that.

## How input flows today (the map)

One GTK main thread runs everything: window input, WebKit UI-process
callbacks, IPC automation, and event fan-out
(`crates/hwatud/src/ipc_server.rs:3`). Key dispatch is two-phase
(`crates/hwatud/src/keys.rs:25`):

- **Capture phase** (modified chords, ctrl/alt): handled on the
  toplevel *before* the WebView sees the key
  (`crates/hwatud/src/window.rs:553-589`). Latency: one GTK dispatch,
  effectively instant.
- **Bubble phase** (bare keys `o`, `/`, `n`...): handled only after
  the WebView *declines* the key
  (`crates/hwatud/src/window.rs:545-551`, `on_window_key` at
  `window.rs:1419`).

The bubble path is where WebKit2's architecture inserts latency: GTK
event propagation is synchronous, but WebKit processes key events in
the web process and reports "handled or not" back to the UI process
asynchronously (WebKit bug 136430 describes the mechanism). Every
bare-key press therefore pays a UI↔web process round trip before
hwatu's own action can run, and that round trip is queued behind
whatever the page's main thread is doing.

## Latency sources, ranked by perceived impact

### 1. Focus-restore dead input (hundreds of ms, worst case seconds)

The RAM-discard strategy (`window.rs:6-11`) destroys the web process
of unfocused windows after `HWATU_DISCARD_SECS` (default 120 s,
`window.rs:26`). Refocusing triggers `restore()`
(`window.rs:953-1008`): take a pooled WebView, deserialize the session
blob, navigate to the current history item. The frozen-frame veil
(`window.rs:978-1006`) hides the *visual* gap well, but input is dead
until the new web process commits: keystrokes and clicks in that
window go nowhere, with no indication. Alt-tab back to hwatu, start
typing into a form, lose the first N keystrokes. This is the single
largest input-lag event a human hits.

Levers, cheapest first:

- **Preemptive restore on pointer entry.** In every tiling WM with
  focus-follows-mouse (and in click-to-focus, where hover precedes
  the click), an `EventControllerMotion::enter` on the toplevel fires
  before `is-active`. Kicking `restore()` from enter would hide most
  of the restore behind the human's own reaction time. Zero cost when
  the window is live (`restore()` early-returns, `window.rs:954`).
- **Preemptive restore on WM-adjacent signals**: `hwatu focus`
  already calls `present()`; the IPC path restores explicitly. The
  gap is purely WM-initiated refocus.
- **Veil should signal input-deadness.** While the veil is up the
  window looks interactive but is not. A one-line "restoring…" strip
  (reuse the bar's Status mode, `bar.rs:33`) would at least convert
  silent keystroke loss into understood waiting.
- **Buffer keystrokes during restore?** Tempting, replaying them into
  the fresh web process is possible via synthesized events, but
  reordering/IME hazards make this a bad trade for a browser that is
  "adequate for a minute". Not recommended.

Measure: time from `connect_is_active_notify` firing to
`LoadEvent::Committed` on the restored view; log it behind an env
flag. Target: hover-triggered restores should complete before the
click lands (>150 ms head start covers the common case).

### 2. Main-loop stalls from automation sharing the input thread

The 2026-07-30 fix (`3f9e97d`) dropped IPC socket reads to
`Priority::DEFAULT_IDLE` (`ipc_server.rs:53-57`), so *accepting* agent
commands no longer preempts input. But the *work* those commands
schedule still runs full main-loop iterations on the input thread:

- **Pixel diff is O(union pixels) on the main thread**
  (`verify.rs:359-380`, plus the row-copy in `Frame::from_texture`,
  `verify.rs:320`). A 1920x1080 diff touches ~2 M pixels x 4 channels
  per call; sweeps (`--viewports`) do it N times. Screenshot *encode*
  was already moved off-thread (`automation.rs:1593`,
  `gio::spawn_blocking`); the diff should follow. `Frame` is plain
  `Vec<u8>` + dimensions, `Send` for free; only texture download must
  stay on the main thread.
- **Snapshot diff LCS is O(n·m) in node count on the main thread**
  (`snapdiff.rs:130-144`, full DP table). Two 2000-node snapshots
  build a 4 M-entry `Vec<Vec<u32>>` while the user types. Options:
  spawn_blocking the diff (nodes are already plain data), or cap
  n·m and fall back to "everything changed" past the cap, which is
  what a huge diff means for the agent anyway.
- **Event fan-out clones the JSON payload per subscriber**
  (`events.rs:88-95`). Fine at current scale; worth remembering if
  console-heavy pages meet several subscribers.

A permanent feedback loop is cheap and worth building: a main-loop
stall watchdog (high-priority `glib::timeout` at ~4 ms measuring
drift; log iterations that overshoot by >8 ms, i.e. a dropped frame
at 120 Hz). That turns "the browser felt sticky" into an attributable
log line and gates regressions the way `bench-spawn.sh` gates spawn
latency.

### 3. Bare-key actions queue behind the page (structural)

`o` (URL bar), `/` (find), `n`/`N` all wait for the web process to
decline the key first (source map above). On a page whose main thread
is busy (heavy JS, long tasks), *hwatu's own leader keys lag*, even
though the daemon is idle. The user reads this as "the browser is
slow", not "the page is slow".

- The phase rule (bare = bubble) is correct as a default: a bare `o`
  typed into a page's text box must reach the page
  (`keys.rs:25-28`).
- But the phase is *derived*, not configurable. A busy-page power
  user has no way to say "I accept that `o` never types into pages;
  give me capture-phase `o`". A `keys.conf` syntax extension
  (`capture+o` or `phase=capture` per line, `keys.rs:335`) is a
  ~20-line change that converts the worst structural latency into an
  explicit user choice. The existing docs sell rebinding to
  ctrl-chords as the workaround; making it a first-class knob is
  more honest.
- Escape in confirm mode is already capture-phase
  (`window.rs:576-587`), good.

Measure: with a fixture page running a 200 ms `while` loop every
second, log press→action latency for `o` vs `ctrl+l`. The delta is
the round-trip tax.

### 4. Find-as-you-type does double web-process work per keystroke

`run_find` fires **both** `count_matches` and `search` on every
`changed` signal of the entry (`window.rs:1642-1659`). Typing "hello"
is 10 web-process operations, each walking the document. The entry
echo itself is GTK-local so typing *feels* fine, but on large
documents the highlight and the counter fight each other and the page
main thread. Debounce `count_matches` (~80 ms trailing) while keeping
`search` immediate: highlight tracks every keystroke, the counter
settles when the user pauses. One `glib::timeout` in `run_find`.

### 5. Scroll keys round-trip a JS eval

`scroll_page` evaluates `window.scrollBy(...)` in the page
(`window.rs:1318-1324`). Correct and simple, but key-repeat
(holding ctrl+shift+j) queues one eval per repeat event; on a janky
page these bunch up and the page keeps scrolling after release, a
classic latency-then-overshoot feel. Two cheap fixes: coalesce (skip
scheduling if the previous scroll eval has not called back), and pass
`behavior: 'instant'` explicitly via `scrollBy({top, behavior})` so
the engine's smooth-scroll animation cannot stack on top of
`enable_smooth_scrolling` (`window.rs:281`) for discrete repeats.
Native wheel-event synthesis would skip JS entirely, but WebKitGTK 6
exposes no scroll adjustment API on the widget, so JS stays the
pragmatic path.

### 6. Engine and compositor level (already mostly right, know the tradeoffs)

- `HardwareAccelerationPolicy::Always` and smooth scrolling are set
  (`window.rs:278-281`), keeping scrolling on the GPU compositor
  path. Right call; the comment already records why forcing
  threaded-scrolling features is *not* done (broke wheel scrolling on
  NVIDIA+Wayland, `window.rs:203-208`).
- `PropagateDamagingInformation` is force-disabled for correctness
  (`window.rs:218`, black-bar artifacts). Cost: full-frame uploads,
  which on weak iGPUs lengthens frame time and therefore input-to-
  photon latency. When the upstream damage fixes (WebKit 305560/
  305758) ship in a stable WebKitGTK, re-enabling this is a latency
  win; the override table makes that a one-line change, and
  `HWATU_WEBKIT_FEATURES=PropagateDamagingInformation:on` lets anyone
  measure today.
- GTK4/GDK compresses pointer motion to the frame clock; up to one
  frame of motion latency is inherent to the toolkit and not
  addressable from hwatu.
- Small unforced main-thread file IO exists on hot paths:
  `finish_discard` writes the session blob synchronously
  (`window.rs:901-910`) and `restore` reads it back
  (`window.rs:963-969`). Blobs are small; acceptable, listed for
  completeness.

## Recommended order of work

| # | change | effort | expected effect | feedback loop |
|---|---|---|---|---|
| 1 | Restore on pointer-enter | S | hides most focus-restore dead time | log is-active→Committed ms |
| 2 | Main-loop stall watchdog | S | makes all other stalls visible | the watchdog *is* the loop |
| 3 | Pixel diff + snapdiff LCS off-thread (or capped) | M | removes worst agent-induced input stalls | watchdog log before/after under `check --baseline` storm |
| 4 | "restoring…" bar strip during veil | S | converts lost keystrokes into understood waiting | manual; no lost-input reports |
| 5 | Debounce find counter | S | halves per-keystroke find work | keystroke→counted-matches trace on a large page |
| 6 | Coalesce scroll evals | S | kills scroll overshoot on janky pages | hold-key release→scroll-stop time |
| 7 | Configurable capture phase in keys.conf | M | opt-out of the bubble round-trip tax | press→action delta on busy fixture |

Not recommended: keystroke buffering across restores (reorder/IME
hazards), forcing threaded scrolling features (known wheel breakage),
a second engine or off-main-thread GTK (not how GTK works).

## Addendum: the 144Hz scroll cap (found and fixed)

Symptom on a 144Hz Wayland laptop (niri, Intel iGPU): idle rAF runs
at ~144fps, but the moment the page actually repaints every frame
(scrolling, animations driving layout) the rate collapses to ~59fps.
GTK was innocent: a bare GTK4 app with a real draw func hits 141fps,
GSK ngl/vulkan/cairo all ~143fps, and GDK reports the monitor's
144003mHz correctly. The `refresh_interval=16.7ms` seen under
`GDK_DEBUG=frames` before the first frame is just the frame-clock
default, not evidence of a 60Hz clock.

The cap lives in WebKitGTK's DMA-BUF presentation path (the default
since 2.40): `AcceleratedSurface` frame completion is paced by
WebKit's own vblank monitor, and when `DisplayVBlankMonitorDRM`
cannot attach to the DRM CRTC it silently falls back to
`DisplayVBlankMonitorTimer`, which ticks at a hardcoded
`FullSpeedFramesPerSecond = 60` (`AnimationFrameRate.h`). Idle rAF
still reads 144 because the UI-side frame clock drives it; only the
produce-a-new-buffer-every-frame loop is throttled.

Measured on the scroll fixture (`scripts/bench-scroll/`), median rAF
delta, `hwatu eval` on the same machine:

| config | idle | scroll |
|---|---|---|
| default (DMA-BUF path) | 62.5-142.9 | 58.8 |
| `GDK_DEBUG=no-vsync` | 142.9 | 142.9 (tears; diagnostic only) |
| `WEBKIT_DISABLE_DMABUF_RENDERER=1` | 142.9 | 142.9 |
| `WEBKIT_DMABUF_RENDERER_FORCE_SHM=1` | 142.9 | 58.8 |
| `GSK_RENDERER=ngl`, `no-offload`, `GDK_DISABLE=offload` | — | 58.8 |

Disabling the DMA-BUF renderer falls back to the legacy EGLImage
path, which presents through the GTK frame clock and therefore
follows the real refresh rate. Verified on Wikipedia (100-142fps
scroll vs 58.8 baseline), CPU cost equal within noise (~13-15%
total during a scroll storm), wheel + keyboard scrolling intact
(ydotool-synthesized wheel deltas land identically), WebGL still
hardware-backed, vsync intact (no tearing).

hwatud now exports `WEBKIT_DISABLE_DMABUF_RENDERER=1` by default
(`main.rs`); explicit user env wins, so
`WEBKIT_DISABLE_DMABUF_RENDERER=0 hwatud` restores WebKit's default
path. Revisit when WebKitGTK's vblank monitor learns to follow the
real display rate (or exposes a refresh-rate override): the DMA-BUF
path is the better architecture and should win again once its pacing
is fixed.
