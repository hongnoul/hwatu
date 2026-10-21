# Hwatu vision

Status: direction of travel. The plan of record is
[`docs/roadmap.md`](docs/roadmap.md). This document defines the principles
roadmap items and platform work must preserve.

## Thesis

Agents need browsers constantly, in parallel, and invisibly. Existing options
each fail one requirement:

- **Automation frameworks** (Playwright, CDP) are built for test suites. A
  page check costs five calls and hundreds of milliseconds.
- **Extensions in the human's browser** share the human's focus, tabs, and
  logins. Every action is one `active: true` from stealing the screen. We
  measured a production bridge stealing focus on three of six core actions.
  The bug class is representable, so it recurs.
- **Cloud browser services** move the session outside the user's trust
  boundary, meter every page, and sell spoofed fingerprints as identity.

Hwatu's answer: the agent gets its own real browser on the user's machine.
Invisible unless summoned. Instant, because it never starts cold. Checks come
back in ~35 ms as measurements, not screenshots to squint at. When a login or
judgment call needs the human, the exact page in progress appears as a window,
the human acts, and the agent continues where they left off. No credential
ever passes through the agent. No cloud.

## The outcome

On Linux, macOS, and Windows alike, an agent can:

1. open a page instantly, invisibly, without touching the human's screen,
2. read, drive, measure, and compare it, and get answers as data,
3. pull the human in to see the exact live page when needed,
4. take it back and continue, nothing lost.

"Cross-platform" means: install normally for the OS, run `hwatu doctor`,
connect an agent, pass the shared verification fixture, perform a live
hand-off. Compiling on three targets is not cross-platform.

## Three planes, one session

Competitors sell these as three products. Hwatu unifies them over the same
live session.

### 1. Verification

The first product. Wins all conflicts. "Pixel-perfect" is a claim;
`match_percent: 97.49` is a measurement. One call opens a page, waits,
reads it, screenshots it, and cleans up. Pixel diffs are scores an agent can
climb. Animations are numbers. Errors are data.

### 2. Identity: delegated authentication

The agent hits a login wall or CAPTCHA. It detects the wall and shows the
human the exact page, live. The human types the password into the real site.
The agent takes the page back, logged in, and stays logged in across
restarts.

The agent never sees the password. The human never gives a password to a
model, a config file, or a cloud. The agent holds only the logins it was
granted, not the user's whole browsing life. What was granted is the
permission model.

Extension bridges expose everything and call it convenience. Cloud services
spoof fingerprints and call it identity. Hwatu earns sessions one hand-off at
a time.

### 3. Hand-off

The same live page moves between agent and human with nothing lost: not the
page, not the login, not half-filled forms. The human enters for one judgment
call or credential, then hands it back. A screenshot of another browser is
not hand-off. Opening the URL elsewhere is not hand-off.

Verification makes the agent trustworthy. Delegated identity makes it capable
on the authenticated web. Hand-off keeps the human sovereign over both.

## Product constitution

More durable than any backend.

- **Verification leads.** It wins shared-runtime conflicts. Its interfaces
  stay evidence-focused.
- **Focus stealing is unrepresentable, not discouraged.** Agent tools reject
  focus verbs. Visibility is a window property, not a launch mode.
  Materialization happens only through the hand-off path. A guideline every
  call site must remember is a bug that has not happened yet.
- **The agent never holds a credential.** Authentication is delegation:
  detect, hand off, human acts, resume. No CAPTCHA solving, no fingerprint
  spoofing, no token injection, no password store.
- **Warm by default.** Daemon, engine, rendering context, and one view stay
  warm between clients.
- **The live session is the hand-off.** Same view, same state, visibility
  toggled.
- **Local-first, no rent.** The daemon runs on the user's machine. No relay
  sees the sessions. No metering shapes agent behavior.
- **One protocol.** CLI, MCP, and integrations are adapters over the
  versioned JSON contract in `hwatu-ipc`.
- **Native engines.** WebKitGTK on Linux, WKWebView on macOS, WebView2 on
  Windows. No bundled Chromium behind the word native.
- **Measured parity.** Engines differ. Hwatu guarantees command semantics and
  reports capabilities explicitly. Pixels compare only against baselines from
  the same declared engine unless the caller opts out.
- **The human side earns its keep.** Since v0.7.0 the human side is a
  credible tiling-WM browser. It shares machinery with verification, and a
  browser the human lives in strengthens hand-off. Tabs, sync, extensions,
  and a password store stay out. Agent loop wins conflicts.
- **Small core, hard boundaries.** Portable code must not import GTK, GLib,
  Cocoa, Win32, or engine types.

## Position against the field

Stated once so roadmap debates do not relitigate it.

| | Hwatu | Extension bridge | Playwright-class | Cloud service |
|---|---|---|---|---|
| Focus safety | unrepresentable | measured 3/6 steal | headed steals always | n/a |
| Warm check | ~35-87 ms, one call | 4-10x slower | ~9x slower as a service | network-bound |
| Identity | delegated per session | inherits whole browser | none | spoofed |
| Parallelism | N headless views | one shared tab strip | good | metered |
| Failure coupling | daemon restarts alone | dies with browser | per-test | vendor outage |
| Trust boundary | user's machine | user's whole identity | user's machine | vendor cloud |

One honest residue: sites that gate on years of accumulated browser history.
Delegated hand-off plus a persistent granted session covers nearly all of
them. The rest is a documented escape hatch, never the default.

## One platform, two products

- **AI verification** owns observation, checks, evidence, agent ergonomics,
  delegated authentication, and session persistence.
- **The tiling-WM browser** owns keyboard navigation, chrome, media, and
  daily-driver usability.

They share the daemon, protocol, backends, sessions, and hand-off. They do
not share product policy. Reusable behavior is promoted into the platform
with a capability contract and conformance coverage, never copied between
products. Plans live in the [roadmap index](docs/roadmap.md).

## What stays portable

Product behavior separates from native mechanism. Names describe
responsibilities, not a refactor plan. Extraction follows working seams and
preserves the wire format.

| Layer | Portable | Native |
| --- | --- | --- |
| `hwatu-ipc` | schema, capability model, compatibility tests | endpoint naming, security |
| client (`hwatu`) | parsing, MCP translation, framing, output | connect/spawn/update/install |
| daemon core | IDs, pools, profiles, quotas, dispatch, session policy, workflows | main loop, process lifecycle |
| verification core | image diff, regions, tolerances, assertions, JSON shaping | pixel capture, engine events |
| browser backend | view/window capability contracts | WebKitGTK, WKWebView, WebView2 |
| platform shell | intent: focus, background, downloads, paths, notifications | GTK/Wayland/X11, AppKit, Win32 |

The backend contract is capability-oriented, not one trait pretending engines
are identical. Minimum explicit operations:

- create, prewarm, navigate, close, focus, unfocus, resize;
- evaluate JavaScript, bridge structured messages;
- observe load, console, network, download, permission, popup, crash;
- capture viewport and full-document pixels;
- inject scripts and content-blocking rules before document code;
- persist session state or truthfully report inability;
- report engine name/version and capability status.

Unsupported behavior returns a stable structured error. It never silently
succeeds, shells out to another browser, or changes meaning by platform.

Granted sessions are the product of a hand-off. They must survive daemon
restarts, be enumerable, and be revocable. A backend that cannot persist says
so in its capability matrix. It does not re-authenticate silently.

## Native platform targets

### Linux

WebKitGTK + GTK is the reference implementation. Wayland/X11 details stay in
the Linux shell. Existing behavior and performance are regression
constraints.

### macOS

WKWebView in an AppKit host. Per-user daemon, protected Unix socket,
Apple-standard directories. Focus/unfocus operates on the same `WKWebView`
without activating the app for agent work. Distribution: signed, notarized
universal package.

Feasibility is measured in [`scripts/probes/macos/`](scripts/probes/macos/);
analysis in
[`docs/research-macos-headless.md`](docs/research-macos-headless.md).

- A CLI binary with `activationPolicy = .prohibited` loads, evaluates, and
  snapshots an offscreen `WKWebView` with no dock icon and no focus steal.
  Warm eval under 1 ms, warm snapshot 2 ms. The warm-service architecture
  transfers.
- **Offscreen `requestAnimationFrame` throttles to 0-1 fps.** No window trick
  recovers it. macOS reports `motion.realtime` degraded. Readiness must never
  depend on frame counts. Deterministic WAAPI seek renders offscreen, so
  verification works; realtime motion requires a visible window.
- `takeSnapshot` is viewport-only. Full-document capture needs an explicit
  strategy.
- A logged-in GUI session appears required. GUI-less CI hosting is unproven;
  verify or document as unsupported.

### Windows

WebView2 in a Win32 host on the Evergreen runtime. Transport: per-user named
pipe, user-restricted ACL. Background work uses a hidden or non-activating
host window whose controller can be shown for hand-off. Distribution: signed
x86_64 and arm64 artifacts, clear WebView2-runtime diagnostics.

## Portability rules

1. Platform dependencies live in target-specific crates/modules and
   dependency tables.
2. Portable crates compile and test on all three CI runners without native
   SDKs.
3. Paths come from platform APIs. No portable module assumes `/tmp`, `$HOME`,
   XDG, a Unix UID, or slash separators.
4. Transport is an interface. Sockets and named pipes carry the same framing,
   timeout, subscription, and authentication semantics.
5. Daemon discovery/spawn is race-safe and idempotent. Concurrent clients
   converge on one healthy per-user daemon.
6. The protocol evolves additively. Capabilities are negotiated before use.
7. Automation semantics are tested against shared fixtures. Backends do not
   fork the CLI or MCP surface.
8. Performance is measured per platform. Linux numbers are not marketed as
   another OS's numbers.

## Known hard problems

Attack early, not after three backends exist.

- **Three UI loops.** GLib, AppKit, and Win32/COM differ in thread and
  reentrancy rules. The core needs a small `spawn_on_ui`/timer contract;
  engine objects stay on their native UI thread.
- **Prewarm may not transfer.** An engine can make view creation opaque. Each
  backend proves warm adoption with measurements or exposes degraded pooling
  honestly.
- **Pixels have provenance.** Scale, color profile, font rasterization,
  scrollbars, and capture APIs differ. Baseline keys include platform,
  engine/version, scale, viewport, color space.
- **Content blocking is asymmetric.** WebKit rule lists and WebView2
  interception differ in coverage and cost. The capability matrix
  distinguishes levels.
- **GUI CI can lie.** Compile-only runners cannot prove non-activation,
  hand-off, DPI, or GPU capture. A physical runner pool owns those gates and
  publishes evidence.
- **Sites gate on accumulated identity.** Hand-off plus persistent sessions
  covers nearly all. Document the residue; never spoof.

## Delivery sequence and exit gates

### M0: freeze the contract

- Golden JSON fixtures for every request, response, event, error.
- Platform/engine metadata and capability negotiation, backward compatible.
- Three CI lanes for portable crates.
- Inventory every GTK, GLib, Wayland/X11, socket, XDG, shell-tool, installer,
  and updater assumption.

**Gate:** compatibility and portable tests pass on all three runners. Every
assumption has an owner and destination.

### M1: extract without changing Linux

- Isolate transport, directories, lifecycle, verification math, engine ops
  behind narrow boundaries.
- Move GTK/WebKitGTK types out of portable orchestration.
- Linux stays continuously working: prewarm, check pool, events,
  focus/unfocus, screenshots, console, downloads, recovery.

**Gate:** Linux e2e suite and benchmarks pass. Portable modules contain no
Linux UI/engine imports. The CLI exercises an in-memory fake backend.

### M2: one vertical slice on macOS and Windows

Smallest complete path per backend: `doctor → discovery → open headless →
wait → eval → screenshot → focus → unfocus → close`.

**Gate:** the path runs on native runners with the same CLI commands and JSON
assertions as Linux, steals no focus before `focus`, and keeps page identity
and session state through hand-off.

macOS scope: verification instrument first. Warm focusless `check`/`diff`,
no browser shell. The tiling-WM browser is not a macOS target: no tiling WM
to serve, and rAF throttling voids scroll-feel work. See
[`docs/research-macos-headless.md`](docs/research-macos-headless.md).

### M3: agent-loop parity

Snapshot/interactables, click/type/scroll/upload, expect, diff/check,
console/network errors, events, profiles, downloads, prompts, popups, crash
recovery, prefetch, pooled reuse, per the capability matrix. Delegated
authentication reaches parity: challenge detection, hand-off, resume, and
persistent granted sessions behave identically wherever claimed.

**Gate:** conformance passes for every supported capability. Unavailable
capabilities fail structurally, with tests. Each platform publishes cold
start, warm connect, open, complete-check latency, idle and per-view memory.

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

**Gate:** two consecutive releases pass conformance, focus-safety, lifecycle,
soak, packaging, and performance gates on all supported architectures. Only
then may README and release metadata claim the platform.

## Swarm execution model

Portability is a dependency graph, not three agents rewriting the daemon.
Long-lived ownership lanes, short integration slices.

| Lane | Owns | Must not own |
| --- | --- | --- |
| Contract | `hwatu-ipc`, capabilities, fixtures, compatibility | backend behavior |
| Core extraction | orchestration, pools, verification interfaces, fake backend | platform implementations |
| Transport/lifecycle | endpoints, discovery, spawn, shutdown, security | protocol payloads |
| Linux guardian | reference backend, regression/performance gates | Linux-convenience API changes |
| macOS backend | WKWebView/AppKit, packaging evidence | shared semantics |
| Windows backend | WebView2/Win32, packaging evidence | shared semantics |
| Conformance | fixtures, focus-safety tests, soak, fault injection | waiving failures |
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
   focus-safety gates. The verification lane reproduces on the target OS.
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

## What we will not do

- Bundle Electron, CEF, or a private Chromium.
- Ride inside the user's daily browser as an extension. We measured that
  architecture's failure modes.
- Store, transmit, or observe user credentials.
- Spoof fingerprints, solve CAPTCHAs, inject tokens, or bypass access
  controls.
- Add a cloud relay between agent and page.
- Replace the protocol with platform-specific CLIs or MCP tools.
- Promise pixel-identical rendering across engines.
- Reduce hand-off to opening a URL in another browser.
- Put browser-shell policy in verification or portable code. Tabs, sync, a
  password store, and an extension platform stay out of scope.
- Block Linux roadmap work on portability foundations.
- Claim support from cross-compilation alone.

## Decision test

A change belongs in Hwatu when it makes the same verification, delegated
identity, or hand-off outcome reliable without weakening the constitution. If
it only makes code look abstract, moves platform details into conditionals,
exposes a credential for convenience, or buys nominal parity by losing
warmth, invisibility, measurement, or session continuity, it is not progress.
