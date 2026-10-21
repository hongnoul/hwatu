# Hwatu vision

Status: direction of travel. The plan of record is
[`docs/roadmap.md`](docs/roadmap.md). This document defines the principles
roadmap items and platform work must preserve.

## Problem

Existing agent browser solutions are bad.

- **Automation frameworks** (Playwright, CDP) are built for test suites. A
  page check costs five calls and hundreds of milliseconds.
- **Extensions in the human's browser** share the human's focus, tabs, and
  logins. Every action is one `active: true` from stealing the screen. We
  measured a production bridge stealing focus on three of six core actions.
- **Cloud browser services** move the session outside the user's trust
  boundary, meter every page, and sell spoofed fingerprints as identity.

## Solution

Agent gets its own real browser on the user's machine. Everything runs
headless: checks, forms, downloads, logins. Checks come back in ~35 ms as
measurements, not screenshots to squint at. For a login, the agent asks the
human for the credential and enters it headlessly; the session persists, so
it is asked once. Nothing ever appears on screen unless the human explicitly
asks to see it. No cloud.

## The outcome

On Linux, macOS, and Windows alike, an agent can:

1. open a page instantly, invisibly, never touching the human's screen,
2. read, drive, measure, and compare it, and get answers as data,
3. ask the human only for what a human must provide,
4. keep what it was granted and continue, nothing lost.

"Cross-platform" means: install normally for the OS, run `hwatu doctor`,
connect an agent, pass the shared verification fixture. Compiling on three
targets is not cross-platform.

## Three capabilities, one session

### 1. Verification

The first product. Wins all conflicts. "Pixel-perfect" is a claim;
`match_percent: 97.49` is a measurement. One call opens a page, waits, reads
it, screenshots it, and cleans up. Pixel diffs are scores an agent can climb.
Animations are numbers. Errors are data.

### 2. Identity

The agent hits a login wall. It asks the human for the credential and enters
it headlessly into the real site. The session persists across restarts; the
human is asked once. The agent holds only the logins it was granted, not the
user's whole browsing life. What was granted is the permission boundary.

No password store, no fingerprint spoofing, no CAPTCHA solving. A challenge
the agent cannot relay as text is reported as a structured block, with the
session intact for whatever the human chooses to do about it.

### 3. Inspection on demand

When the human wants to see what the agent sees, the live session renders on
request: a screenshot, a pixel diff, or a materialized view of the real page.
Seeing is always pull, never push. The session returns to headless with
nothing lost.

Verification makes the agent trustworthy. Granted identity makes it capable
on the authenticated web. Pull-only inspection keeps the human's screen the
human's.

## Product constitution

More durable than any backend.

- **Headless is the invariant, not a mode.** Every capability works without a
  visible window. A feature that requires visibility is out of scope.
- **Nothing appears unsummoned.** Agent tools have no focus verbs. Visibility
  exists only as an explicit human request, and it is a debugging aid, not a
  dependency.
- **Verification leads.** Its interfaces stay evidence-focused.
- **Credentials pass through once.** The human grants a login, the agent
  enters it, the session persists. No password store, no vault, no custody.
- **Warm by default.** Engine and views stay warm between clients.
- **Local-first, no rent.** Everything runs on the user's machine. No relay
  sees the sessions. No metering shapes agent behavior.
- **One protocol.** CLI, MCP, and integrations are adapters over the
  versioned JSON contract in `hwatu-ipc`.
- **Native engines.** WebKitGTK on Linux, WKWebView on macOS, WebView2 on
  Windows. No bundled Chromium behind the word native.
- **Measured parity.** Engines differ. Hwatu guarantees command semantics and
  reports capabilities explicitly. Pixels compare only against baselines from
  the same declared engine unless the caller opts out.
- **Small core, hard boundaries.** Portable code must not import GTK, GLib,
  Cocoa, Win32, or engine types. Idiosyncratic platform code exists only to
  host an invisible view and capture pixels from it.

## Position against the field

Stated once so roadmap debates do not relitigate it.

| | Hwatu | Extension bridge | Playwright-class | Cloud service |
|---|---|---|---|---|
| Focus safety | nothing to steal | measured 3/6 steal | headed steals always | n/a |
| Warm check | ~35-87 ms, one call | 4-10x slower | ~9x slower as a service | network-bound |
| Identity | granted per login | inherits whole browser | none | spoofed |
| Parallelism | N headless views | one shared tab strip | good | metered |
| Failure coupling | daemon restarts alone | dies with browser | per-test | vendor outage |
| Trust boundary | user's machine | user's whole identity | user's machine | vendor cloud |

One honest residue: sites that gate on years of accumulated browser history,
and challenges that cannot be relayed as text. Hwatu reports these as
structured blocks and keeps the session for the human. It does not spoof its
way past them.

## One product

Hwatu is the headless browser for agents. It is not also a browser for
humans. The tiling-WM browser experiments (bar, keybinds, scroll feel, media
chrome) proved the engine renders real pages correctly, and they are done:
that code is pruned, not maintained. Rendering a page for human eyes is an
inspection feature of the agent product, not a second product.

This is a deliberate trade. Human-browser chrome is idiosyncratic by nature:
every line is platform-specific, none of it ports, and it grows superlinearly
with daily-driver ambitions. Headless capability is the portable part. One
product keeps the portable core small and makes three platforms tractable.

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
| browser backend | view capability contracts | WebKitGTK, WKWebView, WebView2 |

The backend contract is capability-oriented, not one trait pretending engines
are identical. Minimum explicit operations:

- create, prewarm, navigate, close, and resize an invisible view;
- evaluate JavaScript, bridge structured messages;
- observe load, console, network, download, permission, popup, crash;
- capture viewport and full-document pixels;
- inject scripts and content-blocking rules before document code;
- persist session state or truthfully report inability;
- report engine name/version and capability status.

Unsupported behavior returns a stable structured error. It never silently
succeeds, shells out to another browser, or changes meaning by platform.

Granted sessions must survive daemon restarts, be enumerable, and be
revocable. A backend that cannot persist says so in its capability matrix. It
does not re-authenticate silently.

## Native platform targets

### Linux

WebKitGTK is the reference implementation. Display-free operation is already
proven: the daemon hosts its own headless compositor when no display exists.
Existing behavior and performance are regression constraints.

### macOS

WKWebView in a minimal host process. Per-user daemon, protected Unix socket,
Apple-standard directories. The probes in
[`scripts/probes/macos/`](scripts/probes/macos/) establish feasibility;
analysis in
[`docs/research-macos-headless.md`](docs/research-macos-headless.md).

- A CLI binary with `activationPolicy = .prohibited` loads, evaluates, and
  snapshots an offscreen `WKWebView` with no dock icon and no focus steal.
  Warm eval under 1 ms, warm snapshot 2 ms. The architecture transfers.
- **Offscreen `requestAnimationFrame` throttles to 0-1 fps.** Readiness must
  never depend on frame counts. Deterministic WAAPI seek renders offscreen,
  so verification works. `motion.realtime` is reported degraded.
- `takeSnapshot` is viewport-only. Full-document capture needs an explicit
  strategy.
- A logged-in GUI session appears required. GUI-less CI hosting is unproven;
  verify or document as unsupported.

Headless-everything removes the hardest macOS problems: no browser shell, no
scroll feel, no visible-window chrome. The verification instrument is the
whole port.

### Windows

WebView2 in a hidden Win32 host on the Evergreen runtime. Transport: per-user
named pipe, user-restricted ACL. Distribution: signed x86_64 and arm64
artifacts, clear WebView2-runtime diagnostics.

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
- **Headless CI can lie.** Compile-only runners cannot prove non-activation,
  DPI, or GPU capture. A physical runner pool owns those gates and publishes
  evidence.
- **Sites gate on accumulated identity.** Granted persistent sessions cover
  nearly all. Report the residue as structured blocks; never spoof.

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
recovery, prefetch, pooled reuse, per the capability matrix. Granted-session
persistence behaves identically wherever claimed.

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

**Gate:** two consecutive releases pass conformance, invisibility, lifecycle,
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

## What we will not do

- Build a browser for humans. No tabs, no bar, no keybinds, no scroll feel,
  no media chrome, no themes, no reader mode, no password manager.
- Bundle Electron, CEF, or a private Chromium.
- Ride inside the user's daily browser as an extension. We measured that
  architecture's failure modes.
- Keep user credentials. Pass-through once, persist the session, never store
  the secret.
- Spoof fingerprints, solve CAPTCHAs, inject tokens, or bypass access
  controls.
- Add a cloud relay between agent and page.
- Replace the protocol with platform-specific CLIs or MCP tools.
- Promise pixel-identical rendering across engines.
- Block Linux roadmap work on portability foundations.
- Claim support from cross-compilation alone.

## Decision test

A change belongs in Hwatu when it makes headless verification or granted
identity more reliable without weakening the constitution. If it requires a
visible window, exposes a credential beyond pass-through, moves platform
details into conditionals, or buys nominal parity by losing warmth,
invisibility, or measurement, it is not progress.
