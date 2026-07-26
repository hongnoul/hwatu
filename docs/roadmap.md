# hwatu roadmap

Status: current as of 2026-07. This file is the plan of record; docs
and marketing should match it.

## The decision

hwatu is **AI-first**. The product is visual verification for coding
agents: a warm WebKit daemon where opening, driving, screenshotting,
and closing a real rendered page costs milliseconds, on the same
machine the human is working on.

hwatu is **not** trying to be a daily-driver browser for humans. That
market belongs to Chromium derivatives on polish and to qutebrowser on
keyboard UX, and competing there would burn months on link hints,
password fill, history, and sync for a niche of a niche. The human
side of hwatu exists to serve exactly one flow: **hand-off**. An agent
drives a headless session, hits something that needs a person (a
CAPTCHA, an ambiguous UI, a visual judgment call), runs
`hwatu focus <id>`, and the same live session materializes in the
user's tiling WM. The human acts, the agent resumes.

Corollary: hwatu should be the best tool in the world at the agent
inner loop, and merely *adequate* at being looked at and poked by a
human for a minute.

## Why this is winnable

- The measured head-to-head ([benchmarks](benchmarks.md)) says raw
  latency is NOT the moat: a warm Playwright server beats hwatu on
  milliseconds, and both are far below agent thinking time. The
  structural advantages are: real WM-mappable windows (headless-shell
  cannot map one at any price), live headed↔headless switching, zero
  Node/browser-download supply chain, and a token-shaped CLI/JSON
  interface.
- Headed/headless as a *window property*, switchable live, is
  structurally impossible for launch-time-headless tools. The human
  hand-off loop is the moat.
- The maintainer dogfoods the agent path daily (jcode native
  backend), so the feedback loop is real. The human path was not
  dogfooded, which is exactly why it stagnated.

## Priorities

### P0: adoption surface

1. **MCP server.** ~~hwatu's best features currently have one consumer
   (jcode).~~ **Shipped:** `hwatu mcp` serves MCP over stdio (no SDK,
   no new dependencies), translating tool calls onto the socket
   protocol, which stays the source of truth. Claude Code, Cursor,
   and other MCP clients adopt hwatu with one config entry.
2. **Published head-to-head benchmark** vs Playwright and
   chrome-devtools-mcp. **Shipped:** `scripts/bench-vs-playwright.mjs`,
   results and honest analysis in [benchmarks.md](benchmarks.md). It
   found real optimization targets: screenshot encode (~90 ms of the
   warm verify pass) and load-settle latency (~50 ms behind Chromium
   on the fixture). Screenshot encode was fixed (threaded fast-PNG,
   ~14 ms). Load-settle tail cost is addressed client-side by
   `--until (committed|dom|settled)` on wait-load/goto/check, and the
   per-step spawn tax by the composite `hwatu check` (one roundtrip
   for open/wait/eval/shot/close); both shipped 2026-07-25 with
   numbers in [benchmarks.md](benchmarks.md).

### P1: the agent-facing "UI" (snapshot quality)

For an agent, the JSON snapshot *is* the interface. Polish it the way
a human browser polishes rendering:

3. **Snapshot diffing.** `hwatu snapshot --diff` returns only what
   changed since the last snapshot of that window. Saves tokens in
   iterate loops.
4. **Stable refs.** Interactable refs that survive re-snapshots of an
   unchanged page, with clear staleness errors on navigation (already
   partially true; make it a documented guarantee).
5. **Assertion primitives.** **Shipped:** `hwatu expect <selector>
   [--text X] [--absent]` polls until the assertion holds (failure
   names what WAS found), and `hwatu diff` covers pixel comparison
   against windows or baselines. **Extended 2026-07-25:** `hwatu
   check --baseline <png> [--tolerance] [--heatmap]` folds the pixel
   tier into the one-roundtrip check, so DOM assertion (`--eval`) and
   visual regression (`--baseline`) are one command, one window, one
   reply.
5b. **Speculative pre-render.** **Shipped 2026-07-25:** `hwatu
   prefetch <url>` starts the load in a headless window and returns
   immediately; the next `check` of the same URL adopts the warm
   window (`"prefetched": true`, `load_ms` ~0). An agent fires it
   right after the edit, thinks/composes while the page loads, and
   the verify step costs ~1 ms instead of the full load. Unclaimed
   prefetches expire after 30 s into the ordinary check pool
   (capped at 3 outstanding), so speculation never raises the memory
   floor. Measured medians on the local fixture: unprefetched check
   84 ms, prefetched check 1 ms ([benchmarks](benchmarks.md)).
5c. **Multi-viewport sweep.** `hwatu check --viewports
   360x640,768x1024,1920x1080` runs the same pass at N sizes
   (sequentially on pooled windows) and reports per-viewport results,
   directly answering the diff envelope's "other widths unverified"
   caveat in one call. Composes with `--baseline-dir` for per-size
   baselines.
6. **Virtual time.** **Prototyped (proto/clock):** `hwatu clock
   pause|resume|step <ms>|set <ms>` puts rAF, `performance.now`,
   `Date.now`, and timers behind one controllable timeline (plus
   CSS/WAAPI from the same clock), so script-driven motion that `seek`
   cannot pin becomes deterministic and diffable. Also the missing
   piece for animation verification in headless windows, where rAF
   and IntersectionObserver never fire natively.

### P2: concurrency and isolation

6. **Profiles.** `--profile <name>`: separate cookie jars/site data
   per profile, so parallel agents (or one agent testing two accounts)
   don't share sessions. One daemon, N isolated headless sessions.
   Extension for parallel-agent infra: derive the default profile
   from the caller's git worktree (hash of the repo root), so N
   agents in N worktrees get isolation with zero flags.
6b. **Client fairness.** One runaway agent must not starve the
   daemon: per-client window quotas and a bounded per-connection
   request rate, with structured "over quota" errors instead of
   silent queueing. Cheap bulkheads before parallel-agent use gets
   heavy.
7. **Display-free operation.** hwatud currently needs a Wayland/X
   session even for headless windows. Promoted to the generative-UI
   workstream as [G4](#g4-display-free-operation-promoted-from-p2-item-7).

### P2: closing the general-automation gaps that matter

Measured against Playwright, hwatu's real coverage gaps are trusted
input, cross-origin iframes, and network visibility. Two of those are
worth native features; the rest stay non-goals.

8. **Trusted input synthesis.** `click`/`type` today dispatch
   synthetic JS events: `isTrusted` is false, and cross-origin iframes
   (hosted payment fields: Stripe Elements, Braintree, Adyen) are
   unreachable from the top frame's JS world entirely. Synthesizing
   input at the GTK/GDK level instead makes events trusted AND lands
   them on whatever is under the pointer, iframes included. One
   feature closes both gaps. Plan: `click --trusted` / `type
   --trusted` (opt-in; the JS path stays default for its landing
   reports), coordinates resolved from the same selector/ref
   machinery. Open question: event delivery into unrealized headless
   windows; may require the offscreen-compositor path from item 7.
   Explicitly NOT an anti-bot evasion feature: it makes real forms
   accept real input, same as every driver-level automation tool.
9. **Network observation (and small-bore stubbing).** An agent
   verifying a form submit should assert "the POST to /api/charge
   returned 200", not squint at a success toast. `console` already
   captures failures (HTTP >= 400); generalize to `hwatu net [--clear]`:
   a structured per-window request log (method, url, status, type,
   timing) from WebKit's resource-load signals. Full Playwright-style
   route interception is out (WebKitGTK does not expose it); if
   stubbing is ever needed for deterministic offline checks, a tiny
   built-in proxy is the honest mechanism, and it stays optional.

### P3: the hand-off loop, productized

10. **Generalized hand-off.** `challenge` already detects CAPTCHAs;
   generalize the pattern: agent flags "needs human" with a reason,
   the window materializes with the reason in the bar, the agent
   awaits resolution. One command pair, any reason.
11. **Hand-off queue.** The async half of item 10: `hwatu handoff
   <id> --reason <text>` queues instead of materializing, `hwatu
   handoffs` lists pending items with reasons, and the human drains
   the queue on their own schedule (each entry promotes its window on
   selection). Respects flow: an agent that needs a human at 14:02
   should not steal focus at 14:02. Log queued-at/answered-at so the
   cost of waiting on a human (and of interrupting one) is a measured
   number, not a vibe.

### P3: context hygiene (snapshot output as a budgeted resource)

Snapshot text goes straight into agent context, so its size and its
trustworthiness are product surfaces:

12. **Budgeted snapshots.** `snapshot --budget <chars>`:
   multiresolution output that degrades coarse-to-fine (landmarks +
   counts, then interactables, then full text) instead of truncating
   arbitrarily. Pairs with snapshot diffing (item 3) to keep iterate
   loops token-flat.
13. **Injection quarantine.** Page text is untrusted input that gets
   pasted into an agent's context. Flag instruction-shaped content
   ("ignore previous instructions", agent-addressed imperatives) and
   move it to a `suspect` field instead of inline text, so a harness
   can drop or fence it. Heuristic and honest about being heuristic:
   a tripwire, not a guarantee. The first verification harness with a
   real answer to snapshot-mediated prompt injection wins trust.

## Workstream: the generative-UI substrate

Thesis (2026-07): agents increasingly *generate* UI per-request
instead of only consuming deployed pages. That future needs a
substrate where every render is instant and machine-verifiable
before a human sees it, which is hwatu's existing pipeline pointed
at a new input source. The wedge (agent verification of the existing
web) funds the platform (render substrate for generated UI).

Architecture rule for everything below: **one pipeline, orthogonal
primitives, composition stays in the caller.** Axes: input (`open
<url>` | `render <html>`), wait (`--until`), observe (eval / snapshot
/ shot / diff / net), notify (one-shot reply | resident push). No
job classification inside the daemon, ever.

Items are ordered; each is scoped to be executable by one focused
session, and each carries the test plan that session must run before
claiming it done. These are bulky: budget more time for validation
than implementation.

### G1. `hwatu render`: documents without a server

**Shipped 2026-07-26.** `hwatu render (--stdin | <file.html>)
[--base <url>]` loads markup directly (`webkit_web_view_load_html`),
composing with every existing check flag (`--eval`, `--shot`,
`--baseline`, `--until`, `--keep`). On the wire it is a `render`
field on `Check` (old clients unaffected; `url` became optional but
serializes identically when present), and an MCP `render` tool.
Inline documents are capped at 8 MiB (`RENDER_MAX_BYTES`), checked
client-side with a clear error and re-checked by the daemon.

The test plan landed as `scripts/test-render.sh` (13 live-daemon
assertions on an isolated socket: stdin/file input, `--base` asset
resolution, `--until dom` on inline scripts, eval/shot/diff on
URL-less windows, session-restore exclusion, pool recycling, 1 MB+
documents, over-cap rejection) plus CLI parse, IPC roundtrip/
back-compat, and MCP minimal-args unit tests. Measured medians in
[benchmarks.md](benchmarks.md): render→shot 96 ms vs 139 ms for
`check` of identical markup over loopback HTTP. Two measured
implementation notes: baseless renders get a unique nonexistent
`file:///hwatu-render/<n>/` base (custom-scheme/unresolvable bases
stall commits 500-700 ms), and the check pool became origin-kind
aware because adopting a file-origin window for an http load forces
a WebKit process swap (~650 ms, worse than a fresh window).

Original test plan (kept for the record):
- Unit: CLI parse tests (stdin vs file vs conflict), MCP minimal-args
  test entry, IPC serde roundtrip.
- Behavioral, live daemon: relative asset resolution against
  `--base`; `--until dom` semantics on inline-script documents;
  eval/shot/diff all work on a rendered (URL-less) window; session
  restore must NOT resurrect rendered windows; window recycling works
  across render→open→render sequences on one pooled window.
- Size: 1 MB+ generated documents through the socket (protocol is
  line-delimited JSON; measure, and cap with a clear error).
- Bench: `render`→shot median vs `check` on a loopback URL serving
  identical markup; render should win (no HTTP). Add to
  benchmarks.md with the usual measured-not-estimated numbers.

### G2. push IPC: subscriptions on a persistent connection

**Shipped 2026-07-26.** A `subscribe` socket request holds its connection
open and streams load lifecycle, console, download, and window events as
JSON lines. Every connection starts with a `subscribed` acknowledgement at
sequence 0, then receives strictly monotonic sequence numbers, optional
`window_id`, timestamps, and kind-specific data. Filters select event kinds
and/or one window. Existing requests retain their one-request/one-reply/EOF
behavior unchanged.

`hwatu watch [--kinds load,console,download,window] [--window ID]` exposes
the stream to shell agents. MCP's `subscribe_events` maps it to
`notifications/hwatu/event`. The GTK thread only performs nonblocking sends
into bounded subscriber channels; a full channel or failed socket removes
that subscriber, so slow and dead clients cannot stall the daemon or retain
an unbounded queue.

The executable plan landed as `scripts/test-watch.sh`: 12 live-daemon
checks cover old one-shot behavior, two concurrent subscribers, event kinds
and monotonic sequences, killed-client cleanup, slow-reader backpressure,
daemon liveness, and filter validation. It passed three consecutive runs.
`scripts/soak-watch.sh` warms through lazy WebKit/GLib initialization before
measuring; a 30-second validation streamed 16,952 events over 1,991 checks,
with RSS 606,576→607,952 KiB and descriptors 46→48. Use a 3,600-second run
for the roadmap's full endurance gate. Spawn median remained 26 ms against
the 60 ms budget; workspace tests, formatting, and clippy were green.

Original test plan (kept for the record):
- Protocol: old single-shot clients against new daemon (byte-for-byte
  same behavior), new client against old daemon (clean error).
- Behavioral: two concurrent subscribers see the same events; a
  subscriber crash mid-stream leaks nothing (assert with daemon
  window/timer counts); events during a window discard/restore cycle;
  slow-reader backpressure (daemon must never block the GTK loop on a
  stuck client socket: write-buffer overflow drops the client, not
  the daemon).
- Soak: 1-hour run with a subscriber attached and a check loop
  hammering; RSS/PSS flat, no fd leaks (`ls /proc/$pid/fd | wc`).

### G3. resident assertions: `expect --watch`

`expect` gains `--watch`: instead of polling inside one eval, install
a MutationObserver-backed monitor that reports over G2 whenever the
assertion's truth value flips. The agent stops paying tokens for the
49 redundant re-checks between the edit and the fix. Compose with the
existing `--visible`/`--text`/`--absent` matchers unchanged.

Test plan:
- Behavioral: flip detection under DOM replacement (framework
  re-render replacing the observed node; the observer must re-arm),
  under navigation (watch dies with a structured event, not
  silence), under virtual-clock pause (must still fire; use the
  native-clock path like the challenge poller).
- Endurance: watch surviving 100 navigations/re-installs without
  duplicate events (sequence numbers strictly monotonic, one event
  per flip).
- Integration: an agent-shaped script (edit fixture → dev-server
  reload → watch event) measuring end-to-end latency from file write
  to event delivery; target < 200 ms.

### G4. display-free operation (promoted from P2 item 7)

Under the substrate thesis this is load-bearing: rendering generated
UI server-side (CI, headless boxes) must not require a logged-in
Wayland session. Evaluate: wlroots headless backend as a managed
child compositor vs WPE WebKit as an alternative backend for
headless-only daemons. Prefer the child-compositor route first (no
second engine to maintain).

Test plan:
- Functional: full test matrix (check/render/eval/shot/diff/clock/
  motion) on a machine/session with no `WAYLAND_DISPLAY`/`DISPLAY`,
  asserting pixel output identical (diff score ≥ 99%) to a
  compositor-hosted run on the same fixture set.
- CI: a GitHub Actions job running the behavioral suite headless.
  This item is done when that job is green on main, because that job
  IS the use case.
- Hand-off boundary: `focus` on a display-free daemon must return a
  structured "no display" error, not crash.

### G5. zero-copy pixels

Screenshot/diff currently round-trip PNG through disk (~14 ms encode
+ write + client re-read). Add a shared-memory path: `shot --shm` /
diff-on-texture, PNG only when a human or model actually consumes
the image. Target: sub-5 ms observed pixels, diff without any encode.

Test plan:
- Correctness: shm pixels byte-identical to the PNG path's decoded
  output on the same frame (freeze with `clock pause` first).
- Bench: remeasure the full verify pass and the vs-Playwright table;
  publish deltas in benchmarks.md.
- Lifecycle: shm segments reclaimed on client death (soak + fd/shm
  accounting), bounded pool so a burst can't exhaust /dev/shm.

### Sequencing and session protocol

Order: G1 → G2 → G3 (needs G2) → G4 → G5. G4 can proceed in parallel
with G2/G3 if two sessions run concurrently; they touch disjoint
code (windowing vs IPC).

Each session working an item should: (1) re-run the existing gates
(fmt/clippy/tests + bench-spawn.sh) before starting, to pin the
baseline; (2) land the item's test plan as automated tests where the
harness allows (unit/parse/protocol tests in-tree; live-daemon
behavioral scripts under `scripts/`); (3) update benchmarks.md with
measured numbers for anything performance-claiming; (4) leave the
one-shot protocol backward compatible: old clients against new
daemons is the invariant that lets sessions ship incrementally.

## The human side: frozen at hand-off quality

Kept and maintained, because the hand-off flow needs them:

- The bar (find, URL entry, y/n prompts), recovery overlays, TLS and
  permission prompts, downloads, crash-restore sessions, keybinds and
  `keys.conf`, the launcher page, adblock.

Explicitly **not planned** (churn-magnet human-browser features):

- Tabs, sync, extensions/WebExtensions, password manager integration,
  link hints, browsing history and URL completion, bookmarks,
  per-site settings UI, DRM (Widevine).

Small human papercuts (zoom, undo-close, yank-URL) are accepted as
patches if they stay inside the existing Action/Keymap machinery, but
they are not roadmap items.

## Non-goals, restated

- Not a scraping browser (Lightpanda's job).
- Not a cross-browser E2E matrix (Playwright's job; hwatu is
  WebKit-only and says so).
- Not a CAPTCHA bypass tool: `challenge` is detection and hand-off
  only, by design.
