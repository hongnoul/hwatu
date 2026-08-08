# AI verification roadmap

Status: current as of 2026-08. This is the plan of record for Hwatu's
agent-facing verification product. Portfolio policy and cross-product priority
live in the [roadmap index](../roadmap.md).

## Outcome

An AI coding agent can open or render a page, drive it, make deterministic
assertions about DOM, pixels, motion, console, and network behavior, then hand
the exact live session to a person when necessary. Agent context cost and
proof quality are first-class product metrics.

Verification code consumes the shared runtime and protocol. It must not import
browser-shell concerns such as keymaps, history UI, or workspace placement.

## Priorities

### P0: adoption surface

1. **MCP server.** ~~hwatu's best features currently have one consumer
   (jcode).~~ **Shipped:** `hwatu mcp` serves MCP over stdio (no SDK,
   no new dependencies), translating tool calls onto the socket
   protocol, which stays the source of truth. Claude Code, Cursor,
   and other MCP clients adopt hwatu with one config entry.
2. **Published head-to-head benchmark** vs Playwright and
   chrome-devtools-mcp. **Shipped:** `scripts/bench-vs-playwright.mjs`,
   results and honest analysis in [benchmarks.md](../benchmarks.md). It
   found real optimization targets: screenshot encode (~90 ms of the
   warm verify pass) and load-settle latency (~50 ms behind Chromium
   on the fixture). Screenshot encode was fixed (threaded fast-PNG,
   ~14 ms). Load-settle tail cost is addressed client-side by
   `--until (committed|dom|settled)` on wait-load/goto/check, and the
   per-step spawn tax by the composite `hwatu check` (one roundtrip
   for open/wait/eval/shot/close); both shipped 2026-07-25 with
   numbers in [benchmarks.md](../benchmarks.md).

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
   **Documented:** [agents.md](../agents.md) now states the guarantee
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
   84 ms, prefetched check 1 ms ([benchmarks](../benchmarks.md)).
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
   virtual time); both are documented in [agents.md](../agents.md).

### P2: closing the general-automation gaps that matter

Measured against Playwright, hwatu's real coverage gaps are trusted
input, cross-origin iframes, and network visibility. Two of those are
worth native features; the rest stay non-goals.

8. **Trusted input synthesis.** **Shipped 2026-08-01:** `click
   --trusted` and `type --trusted` resolve selectors or live refs, calibrate
   page coordinates to the native window, and inject through Linux input
   backends so events arrive with `isTrusted: true`, including targets inside
   cross-origin iframes. The native path is opt-in; the faster JS path remains
   the default and unavailable native backends return structured errors. The
   `examples/trusted-input/` fixture covers top-level and cross-origin targets,
   repeat calls, and text entry. This is for real form compatibility, not
   anti-bot evasion.
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

### P3: context hygiene (snapshot output as a budgeted resource)

**Shipped 2026-08-08** (both items; live checks in
`scripts/test-snapshot-budget.sh`). Item 12 as `snapshot --budget
<chars>` (snapbudget.rs): coarse-to-fine degradation — text halves
to a 200-char floor, interactable fields shorten, entries cap at 30
with an omission marker, final tier is per-tag landmark counts —
with surviving refs keeping their original numbers. Measured:
10807-char full snapshot → 2917 under a 4000 budget. Item 13 as an
instruction-shape tripwire on every snapshot: matching lines move
from `text` into a labeled `suspect` array with a note naming the
heuristic. Honest about being heuristic — a tripwire, not a
guarantee.

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

## Generated-UI verification

Generated documents use the same observation and assertion pipeline as loaded
URLs. Shared transport, display-free operation, and pixel transport are tracked
in the [platform roadmap](platform.md); this roadmap owns the verification
semantics built on them.

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
[benchmarks.md](../benchmarks.md): render→shot 96 ms vs 139 ms for
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
