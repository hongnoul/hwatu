# Benchmarks

Every number below was measured on a real run, not estimated. Rerun
them yourself: the spawn benchmark is `scripts/bench-spawn.sh`, the
rest are a few lines of shell against the release binaries.

**Test rig:** i7-12650H laptop, 15 GiB RAM, Wayland (niri),
WebKitGTK 2.52.5, hwatu built with `cargo build --release`.
Measured 2026-07-19. Page under test: a local 40-card HTML fixture
served by `python3 -m http.server` on loopback.

## Window spawn latency

Time from `hwatu <url>` to a mapped (or realized, for headless),
loading window on a warm daemon. 20 runs per mode, window closed
between runs, 250 ms gap so the prewarm pool refills on the idle path.

| mode | min | median | p90 | max |
|---|---|---|---|---|
| focused (default) | 8 ms | 13 ms | 35 ms | 49 ms |
| `--background` | 13 ms | 16 ms | 16 ms | 16 ms |
| `--headless` | 12 ms | 14 ms | 15 ms | 15 ms |

Notes:

- Background and headless are *steadier* than focused: no compositor
  activation request, so no WM in the timing path.
- The very first window after daemon start pays one-time engine/GPU
  init: 181-407 ms observed. Every window after that is in the table.
- Budget-gated: `scripts/bench-spawn.sh` fails CI-style if the median
  exceeds `HWATU_BENCH_MAX_MS` (default 60).

## Cost of one verification loop

The agent inner loop: open a headless window, wait for the load, read
the DOM, screenshot, close. 10 runs, medians, against the local
fixture page:

| step | median |
|---|---|
| `hwatu --headless <url>` | 13 ms |
| `hwatu wait-load` | 50 ms |
| `hwatu eval 'return document.title'` | 4 ms |
| `hwatu shot /tmp/check.png` (1024x768 PNG) | 142 ms |
| `hwatu close <id>` | 8 ms |
| **whole loop** | **216 ms** |

Two hundred milliseconds per full check, screenshot included. Skip
the screenshot and a DOM-level check (open, wait, eval, close) is
~75 ms. `eval` at 4 ms is cheap enough to poll.

## Memory

Sum of proportional-set-size (PSS) across the daemon and all of its
WebKit child processes (web processes, network process, bwrap
sandboxes, dbus proxies), measured from `/proc/*/smaps_rollup` after
letting each state settle:

| state | total PSS |
|---|---|
| idle daemon + prewarm pool | 457 MB |
| 1 window | 536 MB |
| 5 windows | 757 MB |
| 10 windows | 1016 MB |

That's roughly **56 MB per additional window**, because windows share
one engine, one network process, and one GPU context. For contrast, a
single headless Chromium context via Playwright typically starts in
the 300-500 MB range *per browser*.

The idle floor is WebKit itself (prewarmed WebView, network process,
sandboxes), which is the price of instant spawns. Unfocused windows
are additionally suspended after `HWATU_DISCARD_SECS` (default 120 s),
which kills their web process and returns that ~56 MB until refocus.

## Ad blocking

- **117,431 compiled rules** (EasyList + EasyPrivacy), 655
  inexpressible lines skipped, never approximated.
- **288 ms** to load the compiled ruleset on a warm daemon start
  (compile happens once per list change, ~5 s, then cached).
- **0 JS in the request path**: rules run in WebKit's native
  content-extension engine in the network process, so blocking adds
  no per-window or per-request scripting cost.

## Binary footprint

| binary | size |
|---|---|
| `hwatu` (client, no GTK linkage) | 624 KB |
| `hwatud` (daemon) | 1.1 MB |

## Methodology

- Spawn latency is reported by the daemon itself (request receipt to
  window map/realize) and printed by the client; the harness only
  parses it. See `scripts/bench-spawn.sh`.
- Loop timings are wall-clock around each CLI invocation
  (`date +%s%3N` deltas), so they include client startup, one
  Unix-socket roundtrip, and daemon work: what an agent actually pays.
- Memory is PSS, not RSS, to avoid double-counting the ~300 MB of
  shared WebKit libraries mapped into every child process. RSS sums
  across the tree look 5-6x scarier and are wrong.
- All runs used an isolated `XDG_RUNTIME_DIR` and a fresh daemon so a
  live user session couldn't skew results.

Numbers will vary with hardware, distro WebKitGTK, and compositor.
Include the `hwatud` startup line (webkitgtk version, session,
renderer) when reporting yours.
