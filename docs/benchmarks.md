# Benchmarks

**The headline: a full verification pass — open a page, wait for the
load, read the DOM, screenshot, clean up — costs 24 ms through hwatu
and 83 ms through Playwright, on the same machine, same page, same
clock. A DOM-only check is 19 ms vs 35 ms. And hwatu's side of that
table pays a fresh process spawn per check while Playwright rides a
warm in-process CDP connection: the bias runs against us and we win
anyway.** Full data and every caveat in the
[head-to-head section](#head-to-head-hwatu-vs-playwright--headless-chromium).

Every number below was measured on a real run, not estimated. Rerun
them yourself: the spawn benchmark is `scripts/bench-spawn.sh`, the
head-to-head is `scripts/bench-vs-playwright.mjs`, the rest are a few
lines of shell against the release binaries.

**Test rig:** i7-12650H laptop, 15 GiB RAM, Wayland (niri),
WebKitGTK 2.52.5, hwatu built with `cargo build --release`.
Measured 2026-07-19, remeasured 2026-07-25 after the composite-check
work. Page under test: a local 40-card HTML fixture served by
`python3 -m http.server` on loopback.

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
| `hwatu --headless <url>` | 9 ms |
| `hwatu wait-load` | 49 ms |
| `hwatu eval 'return document.title'` | 2 ms |
| `hwatu shot /tmp/check.png` (1024x768 PNG) | 15 ms |
| `hwatu close <id>` | 6 ms |
| **whole loop** | **87 ms** |

Under ninety milliseconds per full check, screenshot included
(screenshots are encoded off the main loop with fast PNG filtering,
~15 ms). A DOM-level check (open, wait, eval, close) is ~70 ms.
`eval` at 2 ms is cheap enough to poll. Remeasured 2026-07-22 after
the threaded-encode change; the loop was 216 ms before.

### One command instead of five: `hwatu check`

`hwatu check <url> [--eval <js>] [--shot[=path]]` runs the whole loop
above daemon-side in one IPC roundtrip: open headless, wait, eval,
screenshot, close, one JSON reply (url, title, eval result, shot
path, console errors, timings). Finished checks also *recycle*: the
window parks (blanked, console drained, 60 s TTL, max 2) and the next
check navigates it instead of building a new window, which is where
most of the old pass's time went. Measured 2026-07-25 against the
same kind of local fixture, medians over 12 runs:

| variant | median |
|---|---|
| 5-command loop (open, wait, eval, shot, close) | 87 ms |
| `hwatu check --eval ... --shot` (one CLI spawn) | 32 ms |
| same, over the socket (persistent client) | 24 ms |
| `check --eval` only, `--until dom` | 19 ms |

Beyond the wall clock, `check` removes 4 process spawns + 4 socket
roundtrips, the window-leak failure mode (its window always closes or
parks, even on timeout), and 4 tool invocations of agent token cost.
It also bundles the console capture the loop version never read.

### Wait for the stage you need: `--until dom`

`wait-load`, `goto`, and `check` accept `--until
(committed|dom|settled)`. Default stays `settled` (full load, every
subresource). Real pages keep loading images/fonts/third-party JS
long after the DOM is usable; `--until dom` releases at
`DOMContentLoaded`. On a fixture with one 800 ms-slow image
(measured 2026-07-25):

| wait | median |
|---|---|
| `check --until dom` | 68 ms |
| `check --until settled` | 1581 ms |

Both evals saw the identical, fully-parsed DOM (40/40 cards). On
fast-settling real pages the two converge (example.com: 52/52 ms;
a Wikipedia article: 252/271 ms); the win grows with the page's
subresource tail, which is exactly what ad-heavy real sites have.

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
one engine, one network process, and one GPU context. (Chromium's
shared-browser contexts are similarly cheap; see the head-to-head
section below for the honest comparison.)

The idle floor is WebKit itself (prewarmed WebView, network process,
sandboxes), which is the price of instant spawns. Unfocused windows
are additionally suspended after `HWATU_DISCARD_SECS` (default 120 s),
which kills their web process and returns that ~56 MB until refocus.

## Head-to-head: hwatu vs Playwright + headless Chromium

`scripts/bench-vs-playwright.mjs` runs both tools against the same
local fixture page (40 cards, no network), same machine, same clock.
Medians over 12 runs, measured 2026-07-25 (hwatu 83e87ed with the
composite `check` + window recycling, Playwright 1.5x headless-shell
Chromium):

| scenario | hwatu | Playwright |
|---|---|---|
| verify pass, cold engine (start, open, load, eval, shot, teardown) | 392 ms | 176 ms |
| open + full load, warm engine | 82 ms | 21 ms |
| verify pass, warm (5 separate commands: open, load, eval, shot, close) | 87 ms | 83 ms |
| **verify pass, warm (`hwatu check`, one CLI spawn)** | **32 ms** | 83 ms |
| **verify pass, warm (`check` over the socket)** | **24 ms** | 83 ms |
| verify pass, warm, no screenshot (5 separate commands) | 84 ms | 36 ms |
| **DOM verify, no screenshot (`check --until dom` vs `waitUntil:"domcontentloaded"`)** | **19 ms** | 35 ms |
| page-state payload (snapshot JSON vs ARIA snapshot) | 7.3 KB | 5.1 KB |
| memory, 5 pages open (tree PSS, fresh engine) | 813 MB | 259 MB |

Read it honestly: cold start, bare open+load, and memory still go to
Playwright. But the row agents actually live in flipped. The verify
pass — the thing you run hundreds of times while iterating on a page —
is now **32 ms with a screenshot vs Playwright's 83, and 19 ms vs 35
for a DOM-level check**, both through hwatu's worst-case transport (a
fresh CLI process per check) against Playwright's best case (a warm
in-process CDP connection). Two changes did it: `check` collapses the
pass into one roundtrip, and finished checks park their window for
the next one, so steady-state checks skip window construction
entirely (the same trick Playwright's context reuse plays, now on
both sides of the table).

Two widespread claims about the incumbent are simply outdated and
hwatu's docs no longer repeat them: headless-shell Chromium
cold-starts in ~150 ms (not seconds), and 5 shared-browser contexts
cost ~260 MB (not GBs).

What the table does not capture, and why hwatu still exists:

- **Real windows.** hwatu renders every page GPU-composited and
  WM-mappable; headless-shell renders offscreen only. `hwatu focus`
  can hand any session to a human mid-run. Playwright headless has no
  equivalent at any price; headed Chromium costs far more than the
  table shows.
- **No runtime dependency.** hwatu is one static binary + the distro's
  webkitgtk. The Playwright number requires Node, the Playwright
  package, and a ~170 MB browser download per version bump.
- **Token-shaped interface.** hwatu is driven by short CLI commands or
  one-line JSON, no client library or session objects; for coding
  agents the invocation cost (tokens, not milliseconds) is the scarce
  resource.
- **Absolute cost is tiny either way.** 32 ms per screenshot-included
  check is far below any agent's thinking time. The fight is not won
  on stopwatch deltas.

Known optimization targets from this data: cold engine init and the
bare open+load path (window construction dominates it; `check`
sidesteps it via recycling but `open` still pays it). Fixed so far:
screenshot encode (was 90 ms of the pass; threaded fast-PNG encode,
~14 ms), load-settle tail (`--until dom`, 2026-07-25), and per-pass
overhead (composite `check` + window recycling, 2026-07-25, which
took the warm screenshot pass from 83 ms to 24-32 ms). Tracked in
[roadmap.md](roadmap.md).

Caveat on method: hwatu steps go through CLI process spawns (5 per
pass, the worst case for it) except in the socket variants, while
Playwright runs in-process over a persistent CDP connection (its best
case). Numbers for hwatu are therefore ceilings, not floors. Rerun
with `node scripts/bench-vs-playwright.mjs`.

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
