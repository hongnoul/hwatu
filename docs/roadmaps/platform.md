# Shared browser platform roadmap

Status: current as of 2026-08. This is the plan of record for capabilities
shared by the AI verification product and the tiling-WM browser. Portfolio
policy and cross-product priority live in the [roadmap index](../roadmap.md).

## Outcome

One warm daemon owns native web views and exposes one additive, versioned
protocol. Verification and browser-shell features share live sessions without
sharing product policy. A session may move between background agent work and a
visible human window without replacement or state loss.

The durable cross-platform architecture and backend exit gates remain in the
[vision](../../VISION.md). This roadmap tracks product-facing platform work.

## Dependency rule

The platform is the only upstream. When either product discovers a reusable
capability, extract its engine-neutral contract here, add protocol and
conformance coverage, then let both products consume it. Do not copy code
between products and do not make verification depend on the browser shell.

## Concurrency and isolation

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
7. **Display-free operation.** **Shipped 2026-07-30:** headless work can
   run without an existing Wayland/X session. Implementation details and
   regression gates are recorded under [Display-free operation](#display-free-operation).

## Human hand-off

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

## Push IPC

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

## Display-free operation

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

## Zero-copy pixels

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

## Sequencing and gates

As of 2026-08, push IPC and display-free operation are shipped. The remaining
open platform sequence is profiles and client fairness, generalized hand-off
and its queue, then zero-copy pixels. Independent items may proceed in parallel
when they do not alter the same protocol or runtime boundary.

Every platform change must preserve old-client/new-daemon compatibility, run
formatting, clippy, unit tests, and the relevant live-daemon script, and publish
measured deltas for performance claims. Shared capabilities require structured
unsupported errors rather than silent degradation.
