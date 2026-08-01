# hwatu roadmap

Status: current as of 2026-08. This file is the plan of record; docs
and marketing should match it.

Roadmap ordering is reviewed weekly using the evidence and scoring rules in
[continuous-improvement.md](continuous-improvement.md). Repeated user pain and
first-check activation outrank speculative feature breadth.

## The decision

hwatu is **AI-first**. The product is visual verification for coding
agents: a warm WebKit daemon where opening, driving, screenshotting,
and closing a real rendered page costs milliseconds, on the same
machine the human is working on.

**Revised 2026-08:** hwatu is now ALSO becoming a **primary browser
for tiling-WM users** (Hyprland, sway, niri, i3, river). The earlier
position ("the human side exists only to serve hand-off") is
retired. Two things changed it:

1. v0.7.0 proved the model: mainstream keybinds, media-correct video,
   unified shortform controls, and Chromium-curve scrolling landed
   fast because the daemon architecture (one warm engine, ~56 MB per
   extra window, WM-native windows) does the heavy lifting. The
   marginal cost of daily-driver polish is lower than assumed.
2. The demand is real and unserved. Users of the window-per-page
   model say so explicitly ("your web browser shouldn't try to be a
   window manager", "tabs are redundant if my window manager provides
   them") and report abandoning qutebrowser and friends over
   execution polish, not over the model. Nobody owns this niche;
   hwatu's daemon (instant windows from `xdg-open`, one process pool,
   agent hand-off) is structurally the best fit for it.

The hand-off loop stays the moat: an agent drives a headless session,
hits something that needs a person, runs `hwatu focus <id>`, and the
same live session materializes in the user's tiling WM. Daily-driver
quality makes the hand-off destination a browser the human already
lives in, which strengthens the loop rather than competing with it.

Corollary, revised: hwatu should be the best tool in the world at the
agent inner loop, and a **credible primary browser** for the
keyboard-driven tiling-WM user, in that order of priority when they
conflict.

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

3. **Snapshot diffing.** **Shipped 2026-07-30:** `hwatu snapshot
   --diff [--id <id>]` returns only what changed since the last
   snapshot of that window ({added, removed, changed,
   unchanged_count}), diffed via LCS + identity-key pairing so ref
   renumbering is not misreported; per-line text so one edited line
   diffs as one line. First call establishes a baseline (full
   snapshot, `baseline_established: true`); navigation resets it;
   refs stay live handles. MCP snapshot tool gained a `diff` arg.
   `scripts/test-snapshot-diff.sh` (20 checks) covers baseline,
   empty-diff, single-line mutation, live-ref clickability, and
   navigation reset.
4. **Stable refs.** Interactable refs that survive re-snapshots of an
   unchanged page, with clear staleness errors on navigation.
   **Documented:** [agents.md](agents.md) now states the guarantee
   (refs are live element handles; navigation staleness is a clear
   structured error, not a silent mismatch).
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
   **Shipped 2026-07-30:**
   360x640,768x1024,1920x1080` runs the same pass at N sizes
   sequentially on ONE pooled window (resize-reuse measured ~4x
   faster than N separate checks: 15-18 ms vs 66-72 ms for 3 sizes)
   and reports per-viewport results ({size, eval, shot, pass_ms}),
   directly answering the diff envelope's "other widths unverified"
   caveat in one call. Composes with `--baseline-dir` for per-size
   baselines. `scripts/test-viewports.sh` (15 checks) and
   `scripts/bench-viewports.sh` cover it. A pre-existing quirk the
   sweep surfaced (each resize emitted one masked "Script error."
   console entry) was filed as issue #6 and fixed the same day (PR
   #8, daemon-side expression-vs-body syntax check).
6. **Virtual time.** **Shipped 2026-07-23** (merged from
   proto/toolsmith): `hwatu clock
   pause|resume|step <ms>|set <ms>` puts rAF, `performance.now`,
   `Date.now`, and timers behind one controllable timeline (plus
   CSS/WAAPI from the same clock), so script-driven motion that `seek`
   cannot pin becomes deterministic and diffable. Also the missing
   piece for animation verification in headless windows, where rAF
   and IntersectionObserver never fire natively. Shipped surface also
   includes `clock seed <u64>` (deterministic `Math.random`),
   `clock status`, `HWATU_CLOCK_START_PAUSED` for deterministic
   loads, and the companion `hwatu motion [--observe]` (declared
   animation inventory plus model fitting of live motion under
   virtual time); both are documented in [agents.md](agents.md).

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
9. **Network observation (and small-bore stubbing).** **Shipped
   2026-07-30:** `hwatu net [--id <id>] [--clear] [--limit <n>]`
   returns a structured per-window request log (method, final url,
   HTTP status, resource type inferred from response MIME, start_ms
   offset from navigation start, duration_ms) captured from WebKit's
   resource-load signals into a bounded 500-entry ring buffer that
   survives window discards; MCP exposes a matching `net` tool, and
   `scripts/test-net.sh` covers method/status/type capture, 404s,
   POST bodies via fetch, `--clear`, `--limit`, and the cap. Noted
   limitation: WebKitGTK exposes no request destination, so type is
   MIME-inferred, and there is no route interception. Original
   rationale: an agent
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

**Shipped 2026-07-29.** `expect --watch` subscribes before installing a
MutationObserver-backed monitor, reports the initial assertion state and each
truth-value flip as structured `expect` events, then exits after one terminal
navigation event. Existing `--visible`/`--text`/`--absent`/`--contains`/`--nth`
matchers compose unchanged. Native GLib scheduling keeps watches live while the
page's virtual clock is paused, and per-watch sequence numbers make duplicate
or reordered delivery detectable.

The executable plan landed as `scripts/test-expect-watch.sh`: four isolated
live-daemon checks cover initial and flipped state under a paused virtual
clock, framework-style DOM replacement, navigation termination without later
flips, and navigation/reinstall sequence uniqueness. Protocol/CLI/MCP parsing
and monitor-script behavior also have unit coverage; workspace tests, strict
clippy, formatting, and the behavioral suite were green.

Original test plan (kept for the record):
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

### G3.1. scroll-aware multi-point visibility assertions

**Shipped 2026-07-29.** `expect --visible` now scrolls a fully off-screen
match into view before hit testing and samples its center plus four inset
corners. Every sample must resolve to the target or its subtree, so a sticky
header or overlay covering only one edge no longer false-passes. Structured
failures name both the covered sample point and covering element.

`scripts/test-expect-visible.sh` exercises the user-reported cases against a
live isolated daemon: an off-screen target scrolls and passes, a 16 px overlay
covering only its top edge fails with a top-corner diagnostic, and removing
the overlay restores success. Generated resident-watch JavaScript has unit
coverage for all five samples and the scroll path. The first-render workflow
in `docs/agents.md` now teaches agents to establish DOM, rendered, and runtime
invariants before seeding a screenshot baseline.

### G4. display-free operation (promoted from P2 item 7)

**Shipped 2026-07-30.** hwatud detects an unusable/absent
WAYLAND_DISPLAY+DISPLAY at startup and enters display-free mode: it
spawns a managed child headless compositor (probes cage -> labwc ->
sway with WLR_BACKENDS=headless; structured install-hint error if
none), supervises it orphan-free via a PDEATHSIG-holding wrapper
(Linux clears PDEATHSIG on exec of file-caps binaries like distro
sway), and probes /dev/dri/renderD* to fall back to software
rendering (WEBKIT_DISABLE_DMABUF_RENDERER=1, LIBGL_ALWAYS_SOFTWARE=1,
WLR_RENDERER=pixman) on GPU-less boxes. `focus` returns a structured
"no display" error. The CI job "Display-free behavioral (G4)" runs
`scripts/test-display-free.sh` (13 checks incl. 100% pixel parity vs
a compositor-hosted run and orphan checks under quit/SIGKILL) on
ubuntu-latest, green on run 30540003841; Ubuntu 24.04 needs
kernel.apparmor_restrict_unprivileged_userns=0 for WebKit's bwrap
sandbox, set in the job. `scripts/dev/no-gpu.sh` reproduces GPU-less
runners locally.

Original scope (kept for the record). Under the substrate thesis this is load-bearing: rendering generated
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

Order: G1 → G2 → G3 (needs G2) → G4 → G5. Status 2026-07-30: G1, G2,
G3, and G3.1 are shipped; G4 (display-free operation) and G5
(zero-copy pixels) are the open items, and they touch disjoint code
(windowing vs pixel transport) so two sessions can run them in
parallel.

Each session working an item should: (1) re-run the existing gates
(fmt/clippy/tests + bench-spawn.sh) before starting, to pin the
baseline; (2) land the item's test plan as automated tests where the
harness allows (unit/parse/protocol tests in-tree; live-daemon
behavioral scripts under `scripts/`); (3) update benchmarks.md with
measured numbers for anything performance-claiming; (4) leave the
one-shot protocol backward compatible: old clients against new
daemons is the invariant that lets sessions ship incrementally.

## Workstream: the primary browser for tiling WMs

Adopted 2026-08 after a three-track research pass: a code audit of
what exists vs what a daily driver needs, user-testimony research on
why people adopt and abandon minimal keyboard browsers (qutebrowser,
vimb, luakit, nyxt), and a WebKitGTK 2.52 capability audit of what
the engine gives us for free.

The evidence pattern: people adopt this category for keyboard-first
UX, minimal chrome, and config-as-dotfile, and abandon it over (in
order) ad quality, broken sites, password friction, and video. The
window-per-page model itself has vocal demand and no polished
incumbent. So the plan is: fix what is silently broken first, then
ship the category-defining features, then close the abandonment
drivers.

Another session is currently fixing embedded iframe video
interaction; it is deliberately absent from the list below.

### D0: silently broken or one-line-cheap (engine already does it)

Each of these is small because WebKitGTK 2.52 already implements the
hard part; hwatud just never connected the signal or flipped the
setting.

H1. **File uploads.** `run-file-chooser` is unhandled, so `<input
    type=file>` does nothing today. Any upload flow (attaching a file
    to an email, a GitHub issue, a form) is dead. Connect the signal
    to a GTK file dialog (~80 lines). Highest urgency in this tier:
    it is invisible until it bites, then it forces a browser switch.
H2. **WebRTC calls.** `enable-webrtc` defaults OFF; one
    `set_enable_webrtc(true)` plus documenting the `gst-plugins-bad`
    runtime dependency turns on Meet/Discord/Zoom-web calls. The
    permission prompt plumbing already exists. Verify live on
    meet.google.com; per-site UA (`siteua.rs`) covers UA-sniffing
    services.
H3. **Hardware video decode.** Zero code: VA-API decode arrives with
    `gst-plugin-va` (in gst-plugins-bad) + the vendor driver.
    Document it in install/doctor; consider a `hwatu doctor` probe.
    Big battery and smoothness win on laptops, and the same package
    install as H2.
H4. **Web notifications.** Permission prompt exists but
    `show-notification` is unhandled and the Arch build does not link
    libnotify, so grants are silent. Forward to
    `org.freedesktop.Notifications` over D-Bus and route `clicked` to
    window focus (~100 lines).
H5. **Persistent per-site decisions.** Permission grants
    (`prompts.rs` Memory) are daemon-lifetime RAM; every restart
    re-asks mic/cam/notification questions. Persist the
    (host, kind) -> bool map to disk. Add per-site zoom persistence
    on the same store.
H6. **Spell check.** Enchant backend is present; two lines
    (`set_spell_checking_enabled` + languages) plus a config key.
H7. **Printing.** Connect the `print` signal to a
    `WebKitPrintOperation` (~30 lines) and bind Ctrl+P. Print-to-PDF
    comes free.
H8. **PDF viewing.** WebKitGTK ships pdf.js enabled by default;
    verify `decide-policy` does not intercept `application/pdf`
    into a download before the viewer sees it.

### D1: category-defining features (what the audience switches for)

Ordered by user-testimony criticality crossed with implementation
cost. These reverse specific entries in the old not-planned list;
the reversal is deliberate.

H9. **Global history + URL completion.** The single most-missed
    feature in the audit: the bar is a bare GTK Entry and nothing
    records visited URLs, so every navigation is retyped. Store
    (url, title, visit_count, last_visit) in SQLite next to the
    cookie store; fuzzy-complete in the bar and the command palette.
    "Press o and have access to the world" is the retention feature
    of this category.
H10. **Link hints.** Keyboard navigation to links is table stakes
    for the audience (qutebrowser `f`). All the pieces exist: the JS
    injection infra (automation.rs), the interactables enumeration
    (snapshot machinery), and the bar for hint input. Variants can
    wait; plain follow + open-in-new-window + yank-href cover most
    use.
H11. **Password manager integration.** The most-cited "almost
    perfect but..." gap in competitor testimony. First-class fill
    from `pass`, KeePassXC, and Bitwarden CLIs: a fill action that
    shells out, matches by host, and types username/password (TOTP
    next). No sync, no storage of our own, ever.
H12. **Undo close window.** Ctrl+W with no undo loses the page and
    its history; with WM-as-tab-bar this is a daily event. Keep an
    N-deep closed-window stack (URL + serialized history blob, the
    discard machinery already serializes it) and bind Ctrl+Shift+T.
H13. **Quickmarks + search keywords.** Named shortcuts (`:open
    foodrecipes`) and per-engine keywords (`w foo` searches
    Wikipedia) in the existing search.conf/palette machinery. Cheap,
    universally used.

### D2: abandonment drivers (why people go back to Firefox)

H14. **Cosmetic filtering.** Network-level EasyList blocking exists,
    but the #1 stated reason users leave this category is ad quality:
    element hiding is what makes YouTube/news sites bearable.
    Compile EasyList cosmetic rules to per-site injected CSS (the
    content-extension engine handles the network tier already).
H15. **Forced dark mode.** Chromium-derived darkmode is a praised
    qutebrowser feature with no WebKitGTK equivalent; ship a
    prefers-color-scheme override plus an injected-CSS darkener with
    a per-site toggle, persisted on the H5 store.
H16. **Cookie/site-data management.** Persistence exists; clearing
    does not (no verb, no keybind, only hand-deleting cookies.sqlite).
    Add `hwatu clear-site-data [--host H]` and a palette action.
H17. **mpv hand-off.** The loved mitigation for video gaps: a
    keybind that hands the current URL (or hinted link) to `mpv`.
    Trivial with H10's hint machinery.
H18. **Edit-in-$EDITOR.** Celebrated in every browser of this class:
    edit any textarea in the user's editor and paste back on save.
H19. **Session restore to WM workspaces.** Crash-restore exists;
    extend session entries with enough identity (stable per-window
    app_id/title conventions, documented) that WM rules can re-place
    restored windows, and restore on clean quit too (opt-in).

### D3: native-parity shortform scrolling

Adopted 2026-08-01 after a three-agent research pass comparing native
Reels/Shorts/TikTok clients against the mobile-web feeds in hwatu;
full findings in
[research-shortform-native-parity.md](research-shortform-native-parity.md).
The headline: native apps do NOT crossfade between reels — the
seamlessness is commit-time playback handoff plus in-memory adjacent
preload, both replicable from injected user scripts because hwatu
already owns the gesture (smoothwheel's snap pager and synthetic IG
swipe). Ordered by seamlessness per unit effort:

H20. **Commit-time playback handoff.** Native clients start the
    incoming video the moment the gesture commits, under the still-
    running transition, and hard-cut the outgoing audio (tens-of-ms
    ramp against clicks). Web feeds wait for IntersectionObserver
    after settle; hwatu's IG swipe path adds a 650ms absorb on top,
    so audio audibly overlaps. **Shipped 2026-08-01** (smoothwheel
    `handoffPlayback`): at synthetic pointerup / snap-animation
    start, `play()` the incoming card's video and ramp-out+pause the
    outgoing one (volume 1→0 over ~40ms). Idempotent with the site's
    own observer; fail-open. No crossfade: native hard-cuts too.
H21. **Touchpad guard on Instagram Reels.** Precise two-finger
    deltas bypassed the swipeFeed protection (only discrete wheel
    was claimed) and silently desynced IG's gesture-state feed.
    **Shipped 2026-08-01** (smoothwheel `preciseFeedScroll`): precise
    deltas on the IG feed are claimed, accumulated to a flick's worth
    (120px within 300ms), and paged via the same synthetic swipe.
H22. **reelwarm adjacent prefetch.** Native keeps N±1 in memory
    (Media3 PreloadManager). Browser equivalent: user script Range-
    fetches ~1MB (moov + first GOP) of the N±1 cards' video URLs
    into WebKit's network-process cache (IG mobile and TikTok are
    progressive MP4 — warmable), and forces `preload="metadata"` on
    the N+1 element only so GStreamer prerolls one decoded frame.
    Never ±2+: >3-4 concurrent pipelines is the documented deadlock
    zone.
H23. **Settle-curve velocity + extent bounce.** Snap settle is a
    fixed 350ms ease-in-out from zero velocity; native settles start
    at gesture velocity (reuse `easeWithSlope`, ~300ms ease-out).
    Ticks at feed extents are silently eaten; add an iOS-style
    rubber-band transform bounce (c=0.55).
H24. **Shortform verb batch.** Like, share, save, profile, and
    keyboard seek (`,`/`.` = ±2s — IG mobile web has no scrubber at
    all), plus Esc closing the comment sheet. All via the existing
    aria-label matching in smoothwheel.
H25. **MPRIS bridge.** `org.mpris.MediaPlayer2.hwatu` driving the
    shortform play/pause via the existing JS-eval IPC (~150 lines);
    makes playerctl and headset keys work.
H26. **Quality + decode audit.** H3 hardware decode multiplies every
    latency number here. Then: codec advertisement (canPlayType for
    vp9/av1), honest devicePixelRatio (ABR picks rungs by element
    size × dpr), desktop-UA option for youtube shorts (quality menu),
    and verifying IG's ladder under the iPhone UA live.
H27. **Long-session hygiene.** Focused windows never discard, so a
    2h reel session accumulates; add memory telemetry (observe.rs),
    then `WebKitMemoryPressureSettings` and a soft reload-to-current-
    reel-URL past an RSS threshold (shortform URLs carry position,
    so the reload is near-seamless). Same property enables resume:
    persist {url, currentTime} on discard.

Non-gap worth stating: focusshield already beats native on background
audio (WM focus loss never pauses playback), and the compositing path
is measured solid (142.9fps). No crossfade will be added — native
does not have one either.

### Engine-bound gaps (documented honestly, not planned)

- **Widevine/DRM:** WebKitGTK has ClearKey only; no distro ships
  OpenCDM for the GTK port. Netflix/Spotify-web will not play.
  Mitigation: document it; keep a fallback-browser keybind.
- **WebAuthn/passkeys:** not exposed by the GTK port at all. Track
  upstream; sites that hard-require passkeys need the fallback
  browser. This will grow into a real problem and the roadmap should
  re-check upstream status quarterly.
- **Anti-bot walls (Akamai/Cloudflare):** a growing category killer
  for WebKitGTK browsers generally. Per-site UA helps; beyond that
  it is upstream's fight, and `challenge` hand-off is the honest
  answer.

### Still not planned (churn magnets with no constituency here)

- Tabs (the WM tiles), sync, extensions/WebExtensions as a platform
  (specific needs get native features or the userscript escape
  hatch), a password store of our own (we integrate, never store),
  Widevine workarounds.

The old blanket non-goals list claimed link hints, history+completion,
bookmarks, password integration, and per-site settings would never
happen; the daily-driver decision reverses those specific entries,
and this section is the plan of record for them now.

## Non-goals, restated

- Not a scraping browser (Lightpanda's job).
- Not a cross-browser E2E matrix (Playwright's job; hwatu is
  WebKit-only and says so).
- Not a CAPTCHA bypass tool: `challenge` is detection and hand-off
  only, by design.
