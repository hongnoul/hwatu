# Native platforms roadmap

Status: plan of record for taking Hwatu to Linux, macOS, and Windows. The
constraints this plan must preserve live in the [vision](../../VISION.md).
When this plan conflicts with the vision, this plan changes.

## Backend contract

Capability-oriented, not one trait pretending engines are identical. Every
backend must be able to:

- create, prewarm, navigate, resize, and close an invisible view,
- evaluate JavaScript and bridge structured messages,
- observe load, console, network, download, permission, popup, and crash,
- deliver trusted input without touching the user's cursor or focus,
- capture viewport and full-document pixels,
- inject scripts and content-blocking rules before document code,
- persist granted sessions across restarts, list and revoke them, or say
  plainly that it cannot,
- report engine name, version, and capability status.

## Portability rules

1. Platform dependencies live in target-specific crates, modules, and
   dependency tables.
2. Paths come from platform APIs. No portable module assumes `/tmp`, `$HOME`,
   XDG, a Unix UID, or slash separators.
3. Transport is an interface. Unix sockets and named pipes carry the same
   framing, timeouts, subscriptions, and authentication.
4. Daemon discovery and spawn are race-safe and idempotent. Concurrent
   clients converge on one healthy per-user daemon.
5. The protocol evolves additively. Capabilities are negotiated before use.
6. Pixels compare only against baselines from the same declared engine
   unless the caller opts out.
7. Performance is measured per platform. Linux numbers are never marketed as
   another OS's numbers.

## Platform targets

### Linux

WebKitGTK is the reference implementation. Display-free operation is already
proven: the daemon hosts its own headless compositor when no display exists.
Existing behavior and performance are regression constraints.

### macOS

WKWebView in a minimal host process. Per-user daemon, protected Unix socket,
Apple-standard directories. The probes in
[`scripts/probes/macos/`](../../scripts/probes/macos/) establish feasibility.

Probe results (unsigned `swiftc` CLI, offscreen `WKWebView`, activation
policy `.prohibited`):

- Cold snapshot at 1024x768 logical takes 28 to 38 ms at 2x backing scale.
  Warm snapshot takes 2 ms. Warm `evaluateJavaScript` takes under 1 ms.
- Eight concurrent views on one `WKProcessPool` load in 555 ms total.
- Launch to first snapshot is about 610 ms cold. A stripped environment
  (`env -i`) works.
- A cookie written by one process is read by another through
  `WKWebsiteDataStore.default()`.
- WAAPI seek renders the sought state offscreen.
- `requestAnimationFrame` runs at 0 to 1 frames per second offscreen,
  including with `orderFrontRegardless()` and a non-activating panel.
- `launchctl asuser 0` failed, so a session without a logged-in Aqua user
  is unproven.

- A CLI binary with `activationPolicy = .prohibited` loads, evaluates, and
  snapshots an offscreen `WKWebView` with no dock icon and no focus steal.
  Warm eval under 1 ms, warm snapshot 2 ms. The architecture transfers.
- **Offscreen `requestAnimationFrame` throttles to 0-1 fps.** Readiness must
  never depend on frame counts. Deterministic WAAPI seek renders offscreen,
  so verification works. `motion.realtime` is reported degraded.
- `takeSnapshot` is viewport-only. Full-document capture needs an explicit
  strategy.
- A logged-in GUI session appears required. GUI-less CI hosting is unproven.
  Verify it or document it as unsupported.
- The current host (`platform/macos`) is a separate Swift daemon built from
  source. It must ship as a signed binary before macOS is claimed.
- Status: `scripts/test-conformance.sh` passes against it (28 checks, the
  same fixtures as Linux). `scripts/test-reference-skill.sh` passes.
  Trusted input works in headless windows: AppKit events sent to the web
  view arrive with `isTrusted` and do not move the user's cursor or focus.
  Missing: signed package, `net`, `diff`, `motion`.
- A host window below the desktop icons is treated as occluded and its
  page timers drop to about 1 Hz. The host uses normal window level with
  alpha 0.0001.

Headless-everything removes the hardest macOS problems: no browser shell, no
scroll feel, no visible-window chrome. The verification instrument is the
whole port.

### Windows

WebView2 in a hidden Win32 host on the Evergreen runtime. Transport: per-user
named pipe with a user-restricted ACL. Distribution: signed x86_64 and arm64
artifacts with clear WebView2-runtime diagnostics.

- WebView2 ships with Windows 10 and 11, so the host stays a few MB with no
  bundled engine.
- Trusted input can go through `CallDevToolsProtocolMethod`
  (`Input.dispatchMouseEvent`, `Input.dispatchKeyEvent`), which produces
  `isTrusted` events without moving the user's cursor.
- Hidden, not minimized: minimized hosts get timer throttling. Prove this in a
  spike before broad parity work.
- Status: `crates/hwatu` and `hwatu-ipc` build and pass clippy for
  `x86_64-pc-windows-msvc`. The default endpoint is the named pipe
  `\\.\pipe\hwatu-<user>`. No WebView2 host exists yet, so nothing
  listens on it.

## Known hard problems

Attack early, not after three backends exist.

- **Three UI loops.** GLib, AppKit, and Win32/COM differ in thread and
  reentrancy rules. The core needs a small `spawn_on_ui`/timer contract.
  Engine objects stay on their native UI thread.
- **Prewarm may not transfer.** An engine can make view creation opaque. Each
  backend proves warm adoption with measurements or exposes degraded pooling
  honestly.
- **Pixels have provenance.** Scale, color profile, font rasterization,
  scrollbars, and capture APIs differ. Baseline keys include platform,
  engine/version, scale, viewport, and color space.
- **Content blocking is asymmetric.** WebKit rule lists and WebView2
  interception differ in coverage and cost. The capability matrix
  distinguishes levels.
- **Headless CI can lie.** Compile-only runners cannot prove non-activation,
  DPI, or GPU capture. A physical runner pool owns those gates and publishes
  evidence.
- **Sites gate on accumulated identity.** Granted persistent sessions cover
  nearly all of it. Report the residue as structured blocks. Never spoof.
- **Page-side scripts drift.** Snapshot ranking, `--contains` matching, and
  content extraction must be one shared asset loaded by every backend, not
  three reimplementations. The stop classifiers and the challenge detector
  are already shared from `page-scripts/`.

## Delivery sequence and exit gates

### M0: freeze the contract

- Golden JSON fixtures for every request, response, event, and error.
- Platform/engine metadata and capability negotiation, backward compatible.
- Three CI lanes for portable crates.
- Inventory every GTK, GLib, Wayland/X11, socket, XDG, shell-tool, installer,
  and updater assumption.

**Gate:** compatibility and portable tests pass on all three runners. Every
assumption has an owner and destination.

### M1: extract without changing Linux

- Isolate transport, directories, lifecycle, verification math, and engine ops
  behind narrow boundaries.
- Move GTK/WebKitGTK types out of portable orchestration.
- Linux stays continuously working: prewarm, check pool, events, screenshots,
  console, downloads, recovery.

**Gate:** Linux e2e suite and benchmarks pass. Portable modules contain no
Linux UI/engine imports. The CLI exercises an in-memory fake backend.

### M2: one vertical slice on macOS and Windows

Smallest complete path per backend: `doctor → discovery → open headless →
wait → eval → screenshot → close`.

**Gate:** the path runs on native runners with the same CLI commands and JSON
assertions as Linux, and never activates an app or shows a window.

### M3: agent-loop parity

Snapshot/interactables, click/type/scroll/upload, expect, diff/check,
console/network errors, events, profiles, downloads, prompts, popups, crash
recovery, prefetch, and pooled reuse, per the capability matrix. Granted
sessions persist identically wherever claimed.

**Gate:** conformance passes for every supported capability. Unavailable
capabilities fail structurally, with tests. Each platform publishes cold
start, warm connect, open, complete-check latency, idle memory, and per-view
memory.

### M4: native installation

- Linux: distro packages, explicit WebKitGTK diagnostics.
- macOS: signed/notarized universal artifacts, launch integration.
- Windows: signed x86_64/arm64 artifacts, named-pipe lifecycle, WebView2
  diagnostics.
- `setup`, `doctor`, `update`, uninstall: previewable, idempotent,
  reversible.

**Gate:** a clean OS image installs, connects MCP, runs the smoke fixture,
updates, and uninstalls without source tools or undocumented steps.

### M5: declare support

**Gate:** two consecutive releases pass conformance, invisibility, lifecycle,
soak, packaging, and performance gates on all supported architectures, and
one unmodified reference skill (an authenticated, multi-step consumer flow
stopped before payment) completes from a clean install on each OS. Only then
may README and release metadata claim the platform.

## Swarm execution model

Portability is a dependency graph, not three agents rewriting the daemon.
Long-lived ownership lanes, short integration slices.

| Lane | Owns | Must not own |
| --- | --- | --- |
| Contract | `hwatu-ipc`, capabilities, fixtures, compatibility | backend behavior |
| Core extraction | orchestration, pools, verification interfaces, fake backend | platform implementations |
| Transport/lifecycle | endpoints, discovery, spawn, shutdown, security | protocol payloads |
| Linux guardian | reference backend, regression/performance gates | Linux-convenience API changes |
| macOS backend | WKWebView implementation, packaging evidence | shared semantics |
| Windows backend | WebView2 implementation, packaging evidence | shared semantics |
| Conformance | fixtures, invisibility tests, soak, fault injection | waiving failures |
| Release | signing, artifacts, clean-image lifecycle | claiming parity without evidence |

### Rules

1. **Contract first.** A failing test and capability expectation precede every
   native feature assignment.
2. **Vertical slices.** One end-to-end operation at a time: implementation,
   fixture evidence, failure semantics, diagnostics.
3. **One writer per boundary.** Others propose via tests or an interface
   request.
4. **Linux stays green.** Extraction lands with demonstrated unchanged
   behavior and benchmark deltas.
5. **Two-sided gates.** Implementers do not approve their own conformance or
   invisibility gates. The verification lane reproduces on the target OS.
6. **Capability honesty.** Missing parity becomes a matrix entry and a
   runnable test, never a hidden conditional.
7. **Small cadence.** Protocol/core first, then rebase platform slices. No
   platform mega-branches.
8. **Evidence is the hand-off.** Each task reports commands, runner and
   engine versions, fixture output, performance delta, capability gaps, and
   files not inspected.

First wave:

```text
Contract ─────┬─> transport/lifecycle ─┬─> macOS vertical slice
              │                        └─> Windows vertical slice
              ├─> fake backend + core extraction
              └─> conformance harness ─┬─> Linux regression gate
                                      ├─> macOS verification gate
                                      └─> Windows verification gate
```

No platform lane starts broad parity until M1's fake backend and Linux
regression gate make the boundary executable.
