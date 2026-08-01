# Benchmarks

**The headline: hwatu ships warm.** A full verification pass (open a
page, wait for the load, read the DOM, screenshot, clean up) is one
command and **35-39 ms** through hwatu, vs **82 ms** through a warm
in-process Playwright connection, on the same machine, page, and
clock. A DOM-only check is **21 ms vs 49 ms**. And when Playwright is
used the way hwatu is used, a fresh client process talking to a
kept-warm browser server, the shape every shell-driven agent actually
has, its pass costs **341 ms**: hwatu's architecture is ~9x faster
at being a warm service, because being a warm service is the whole
design. Full data and every caveat in the
[head-to-head section](#head-to-head-hwatu-vs-playwright--headless-chromium).

Every number below was measured on a real run, not estimated. Rerun
them yourself: the spawn benchmark is `scripts/bench-spawn.sh`, the
head-to-head is `scripts/bench-vs-playwright.mjs`, the token/context
budget check is `scripts/bench-tokens.mjs`, and the rest are a few
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
| 5-command loop (open, wait, eval, shot, close) | 103 ms |
| `hwatu check --eval ... --shot` (one CLI spawn) | 39 ms |
| same, over the socket (persistent client) | 35 ms |
| `check --eval` only, `--until dom` | 21-22 ms |

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

### Pay the load while you think: `hwatu prefetch`

`hwatu prefetch <url>` starts the load in a headless window and
returns immediately; the next `check` of the same URL adopts the
warm window instead of navigating (`"prefetched": true` in the
reply). Measured 2026-07-25 on a local fixture, `check` wall-clock
totals over 10 runs each, 0.5 s of simulated agent think time
between prefetch and check:

| variant | runs (ms, sorted) | median |
|---|---|---|
| `check` (pooled window, no prefetch) | 6 7 7 61 80 88 90 95 96 99 | 84 ms |
| `prefetch`, think 0.5 s, `check` | 1 1 1 1 1 1 1 2 2 2 | 1 ms |

The unprefetched spread shows the two regimes: sub-10 ms when the
page was still warm in the recycled window, ~60-100 ms when it
actually navigated. Prefetch pins the fast regime: the load runs
during think time, so the check pays only adoption + eval. Unclaimed
prefetches expire after 30 s into the ordinary check pool (max 3
outstanding), so speculation never raises the daemon's memory floor,
and a check with no matching prefetch just loads normally.

### Documents without a server: `hwatu render`

`hwatu render (--stdin | <file.html>)` loads markup directly
(`webkit_web_view_load_html`) instead of navigating to a URL, with
the same one-roundtrip pass as `check` (`--eval`, `--shot`,
`--baseline`, `--until`, `--keep`). An agent holding generated HTML
skips the temp-file-plus-`http.server` dance entirely. Measured
2026-07-26 (`scripts/bench-render.sh`, 40 runs, identical markup for
both paths, `--shot` included):

| variant | median |
|---|---|
| `render` (inline markup, no HTTP) | 96 ms |
| `check` (same markup over loopback HTTP) | 139 ms |

Render wins by skipping the HTTP roundtrip and server, but the real
value is operational: nothing to serve, nothing to clean up.

Two measured cliffs shaped the implementation:

- **The default base URI must be cheap.** `load_html` against an
  unregistered custom scheme or an unresolvable http base stalled
  the commit 500-700 ms in the network process; `file:`/`about:`
  bases commit in single-digit ms. Baseless renders therefore get a
  unique `file:///hwatu-render/<n>/` base (nonexistent path, so
  relative references resolve to nothing rather than to real files).
- **The check pool is origin-kind aware.** WebKit swaps web
  processes when a navigation crosses the file:/network boundary,
  and adopting a file-origin (rendered) pool window for an http
  check cost ~650 ms vs ~240 ms for a fresh window. Parked windows
  remember their origin kind, and a check only adopts a matching
  park, so alternating render/check loops keep one warm window per
  kind instead of thrashing process swaps.

## Tokens per verification

Measured 2026-08-01 with `scripts/bench-tokens.mjs` against the same
40-card local fixture used by the latency benchmarks. This benchmark
measures the text a coding agent has to ingest from browser-verification
tool output. It always reports tokenizer-independent UTF-8 bytes, then
optionally reports one pinned tokenizer, `gpt-tokenizer`'s `cl100k_base`
encoding, so readers can compare runs without pretending every model
uses that tokenizer.

| transcript | source | UTF-8 bytes | `gpt-tokenizer` `cl100k_base` tokens |
|---|---:|---:|---:|
| `hwatu-live-check-json` | live `hwatu check` output against the fixture | 306 | 103 |
| `hwatu-check-json-fixture` | built-in representative hwatu fixture output | 342 | 127 |
| `playwright-mcp-input-template` | local transcript input template, not a measurement | 256 | 55 |
| `chrome-devtools-mcp-input-template` | local transcript input template, not a measurement | 266 | 58 |

The two MCP competitor rows are deliberately **not** external
measurements. The tools were not available in this run, so the script
ships rerunnable input slots instead of fabricated numbers:

```sh
npm install --prefix /tmp/hwatu-tokenizer --no-save gpt-tokenizer
NODE_PATH=/tmp/hwatu-tokenizer/node_modules PATH=$PWD/target/release:$PATH \
  node scripts/bench-tokens.mjs \
    --hwatu-live \
    --input playwright-mcp=bench-inputs/playwright-mcp.txt \
    --input chrome-devtools-mcp=bench-inputs/chrome-devtools-mcp.txt
```

Paste each competitor's actual tool transcript for the same fixture into
the named input file before running that command. The script reports bytes
and pinned-tokenizer counts for every transcript, but its failure gate
applies **only** to hwatu rows (`HWATU_TOKEN_BENCH_MAX_BYTES`, default
16,384, and `HWATU_TOKEN_BENCH_MAX_TOKENS`, default 4,096 when the
optional tokenizer is installed). Competitor payloads are comparison
inputs, not CI failure criteria for hwatu.

Caveats:

- Bytes are the stable, tokenizer-independent measurement. Token counts
  vary by model vendor, chat wrapper, tool-call envelope, and tokenizer
  revision.
- The pinned tokenizer is an OpenAI-style `cl100k_base` proxy, not a
  claim about Claude, Gemini, or any MCP host's exact billing tokenizer.
- Screenshot bytes are not included; this measures the textual tool
  output an agent must read after a verification pass.
- The live hwatu row was produced after `cargo build --release` with
  `PATH=$PWD/target/release:$PATH`; installed-release output can drift as
  fields are added or removed.

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
Medians over 16 runs, measured 2026-07-25 (hwatu 83e87ed with the
composite `check` + window recycling, Playwright 1.5x headless-shell
Chromium):

| scenario | hwatu | Playwright |
|---|---|---|
| verify pass, cold engine (start, open, load, eval, shot, teardown) | 435 ms | 190 ms |
| open + full load, warm engine | 96 ms | 26 ms |
| verify pass, warm (5 separate commands: open, load, eval, shot, close) | 103 ms | 82 ms |
| **verify pass, warm (`hwatu check`, one CLI spawn)** | **39 ms** | 82 ms |
| **verify pass, warm (`check` over the socket)** | **35 ms** | 82 ms |
| **verify pass as a warm *service* (fresh client process → warm engine)** | **39 ms** | 341 ms |
| verify pass, warm, no screenshot (5 separate commands) | 93 ms | 42 ms |
| **DOM verify, no screenshot (`check --until dom` vs `waitUntil:"domcontentloaded"`)** | **21 ms** | 49 ms |
| page-state payload (snapshot JSON vs ARIA snapshot) | 7.3 KB | 5.1 KB |
| memory, 5 pages open (tree PSS, fresh engine) | 774 MB | 260 MB |

The rows mean different things, so read them separately:

- **Warm in-process (82 ms)** is Playwright at its best: a Node
  program holding a live CDP connection. If your agent IS a
  long-running Node process, this is what it pays. hwatu's `check`
  still beats it 2x+, *through a fresh CLI process per call*.
- **Warm service (341 ms)** is Playwright shaped like hwatu: engine
  kept warm in `launchServer()`, each check a fresh client that
  connects and disconnects, which is what "keep Playwright warm"
  means for any shell-driven agent, CI step, or MCP tool that
  shells out. Node startup + WebSocket connect + remote context
  creation eat 300 ms before any browsing happens. hwatu's whole
  design is being that warm service: one Unix socket roundtrip, 39
  ms, ~9x faster. Playwright is a library that must be *made* warm;
  hwatu is a daemon that cannot be cold (first client spawn
  autostarts it).
- **Cold engine (190 vs 435 ms)** still goes to Playwright, paid
  once per boot on hwatu's side, once per script invocation for
  library-style Playwright use.

Two widespread claims about the incumbent are simply outdated and
hwatu's docs no longer repeat them: headless-shell Chromium
cold-starts in ~150-200 ms (not seconds), and 5 shared-browser
contexts cost ~260 MB (not GBs).

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
- **Absolute cost is tiny either way.** 39 ms per screenshot-included
  check is far below any agent's thinking time. The fight is not won
  on stopwatch deltas.

Known optimization targets from this data: cold engine init and the
bare open+load path (window construction dominates it; `check`
sidesteps it via recycling but `open` still pays it). Fixed so far:
screenshot encode (was 90 ms of the pass; threaded fast-PNG encode,
~14 ms), load-settle tail (`--until dom`, 2026-07-25), and per-pass
overhead (composite `check` + window recycling, 2026-07-25, which
took the warm screenshot pass from ~100 ms to 35-39 ms). Tracked in
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
