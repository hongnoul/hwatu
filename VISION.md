# Hwatu vision

Status: direction of travel. The current plan of record remains
[`docs/roadmap.md`](docs/roadmap.md). This document defines the product
principles that roadmap items and platform work must preserve.

## The thesis

An AI agent needs a browser the way a compiler needs a filesystem: constantly,
in parallel, and without asking anyone's permission to look. Every existing
answer gets one of three things wrong:

- **Automation frameworks** (Playwright, CDP drivers) are cold, verbose, and
  built for test suites, not for an agent's inner loop. Five calls and
  hundreds of milliseconds to answer "does the page look right?"
- **Extensions riding in the human's browser** share one window manager, one
  tab strip, and one focus with a person who is trying to work. Every action
  in that architecture is one `active: true` away from stealing the screen.
  This is not hypothetical: we measured a production agent bridge stealing
  focus on three of its six core actions, including every screenshot of a
  background tab. The bug class is *representable*, so it recurs.
- **Cloud browser services** solve isolation by moving the browser away from
  the machine, the session, and the user's trust boundary, then charge rent
  on every page load and call fingerprint-spoofing "verified identity."

Hwatu's answer: one warm local daemon owning real native web views, headless by
construction, driven over one JSON protocol, with the live session itself as
the unit of hand-off between agent and human. Verification in ~35 ms, focus
stealing unrepresentable, identity granted by the human without ever exposing
a credential, and no cloud in the loop.

## The outcome

Hwatu should give an AI coding agent the same loop on Linux, macOS, and
Windows:

1. connect to one warm local daemon,
2. open or adopt a prewarmed native web view without taking focus,
3. inspect, drive, measure, and compare the rendered page through the same
   JSON protocol,
4. materialize that exact live session when a person is needed, and
5. return it to the background without losing state.

"Cross-platform" does not mean that the Rust workspace happens to compile on
three targets. It means a user can install Hwatu in the normal way for their
OS, run `hwatu doctor`, connect an agent through CLI or MCP, complete the same
verification fixture, and perform a live human hand-off.

## Three planes, one session

Hwatu is best understood as three capabilities that competitors treat as three
products, unified by the fact that they all operate on the *same live
session*:

### 1. Verification: evidence instead of vibes

The first product and the tiebreaker in every conflict. "Pixel-perfect" is a
claim; `match_percent: 97.49` is a measurement. One `check` call opens,
waits, evaluates, screenshots, and closes in a single IPC roundtrip. Pixel
diffs are scores an agent can climb. Animations are numbers. Console and
network errors arrive as data. The agent's inner loop runs at memory speed,
in parallel, invisibly.

### 2. Identity: delegated authentication, not credential sharing

The under-told story. When an agent hits a login wall or a CAPTCHA, the
`challenge` / `focus <id>` / resume loop turns authentication into a
**delegation protocol**:

- the agent detects the wall and materializes the same live window for the
  human,
- the human types the password or solves the challenge *in a real window,
  into the real site*,
- the agent resumes the same session, cookies intact, and keeps it across
  daemon restarts in its own persistent profiles.

The agent never sees the credential. The human never shares a password with a
model, a config file, or a cloud. And unlike an extension in the daily
browser, the agent holds *only* the sessions it was explicitly granted, not
the user's entire logged-in life. Session scope is the permission model.

This inverts the industry default. Extension bridges expose everything and
call it convenience; cloud services spoof fingerprints and call it identity.
Hwatu's sessions are earned one hand-off at a time, which is both the safest
and the most honest version of "the agent can act as me."

### 3. Hand-off: the live session is the interface between agent and human

Focus and unfocus preserve page, cookies, navigation, and in-progress work in
both directions. The agent works headless; the human can be pulled in for
exactly one judgment call or one credential and then hand the session back. A
screenshot of a replacement browser is not hand-off. Opening the URL in
another browser is not hand-off. Same view, same session, visibility as a
property.

Each plane is independently useful. Together they compound: verification makes
the agent trustworthy, delegated identity makes it capable on the
authenticated web, and hand-off keeps the human sovereign over both.

## Product constitution

These decisions are more durable than any backend.

- **Agent verification leads.** AI verification is the first product and wins
  shared-runtime conflicts. Its interfaces stay focused on reproducible
  evidence rather than absorbing general browser UI.
- **Focus stealing is unrepresentable, not discouraged.** Headless and
  background work must not steal focus, and the architecture must make the
  violation impossible to express: agent-facing tools reject focus verbs,
  visibility is a live window property rather than a launch mode, and
  materialization happens only through the human-facing hand-off path. A
  guideline that every call site must remember is a bug that has not happened
  yet. We fixed exactly this class in an extension-based bridge; Hwatu must
  never be able to host it.
- **The agent never holds a credential.** Authentication is delegation:
  challenge detection, hand-off, human action, resume. Hwatu does not solve
  CAPTCHAs, spoof fingerprints, inject tokens, or store the user's passwords.
  Sessions the human grants are kept; secrets are never seen.
- **Warm by default.** The daemon, engine, rendering context, and one useful
  view remain warm between short-lived clients.
- **The live session is the hand-off.** Focus and unfocus preserve page,
  cookies, navigation, and in-progress work. A screenshot of a replacement
  browser is not hand-off.
- **Local-first, no rent.** The daemon runs on the user's machine, inside the
  user's trust boundary. No cloud relay sees the user's sessions, and no
  per-page metering shapes the agent's behavior.
- **One protocol.** CLI, MCP, and native integrations remain adapters over the
  versioned newline-delimited JSON contract in `hwatu-ipc`.
- **Native engines, portable behavior.** Use the OS engine and window stack:
  WebKitGTK on Linux, WKWebView on macOS, and WebView2 on Windows. Do not
  hide a bundled Chromium download behind the word native.
- **Measured parity, not claimed parity.** Web engines differ. Hwatu
  guarantees command semantics and reports engine/platform capabilities
  explicitly. Pixel output is only compared against a baseline captured for
  the same declared engine unless the caller opts into another policy.
- **The human side earns its keep, then some.** Since v0.7.0 the human side is
  a credible primary browser for tiling WMs: mainstream keybinds,
  media-correct playback, unified shortform controls, Chromium-curve
  scrolling. The [roadmap](docs/roadmap.md) has since adopted this as a real
  second audience: the daily-driver polish is cheap because it shares
  machinery with agent verification, and a browser the human already lives in
  makes the hand-off stronger, not weaker. The true churn magnets (tabs,
  sync, an extension platform, a password store of our own) stay out. When
  agent and human needs conflict, the agent inner loop wins.
- **Small core, hard boundaries.** Portable orchestration and verification
  math must not import GTK, GLib, Cocoa, Win32, or engine-specific types.

## Position against the field

Stated once, so roadmap debates do not relitigate it.

| | Hwatu | Extension in the user's browser | Playwright-class driver | Cloud browser service |
|---|---|---|---|---|
| Focus safety | unrepresentable by design | one `active:true` from regression, measured in production | headed mode steals always | n/a (remote) |
| Check latency, warm | one call, ~35-87 ms | 4-10x slower through host/extension hops | ~9x slower as a warm service | network-bound |
| Identity model | delegated per-session, credential never seen | agent inherits the user's entire logged-in browser | fresh profiles, no story | spoofed fingerprints, rented trust |
| Parallelism | N headless views, no contention | one shared tab strip and focus | good | metered |
| Failure coupling | daemon restarts alone | dies with the host browser | per-test | someone else's outage |
| Trust boundary | user's machine | user's machine, user's whole identity | user's machine | vendor's cloud |

The extension column is not a strawman: where the user must be *recognized* by
a site through years of accumulated browser history, an identity-bearing
bridge still has a narrow edge. Hwatu's answer is one hand-off per site and a
persistent granted session, which covers that slice in practice while keeping
the permission model scoped. If a hard case remains, it is a deliberate
escape hatch, never the default plane.

## One platform, two products

The shared native-browser platform supports two independently judged products:

- **AI verification** owns machine-readable observation, deterministic checks,
  evidence, and agent ergonomics. Delegated authentication and session
  persistence live here: they are agent capabilities, not browser chrome.
- **The tiling-WM browser** owns keyboard navigation, human-facing chrome,
  media behavior, and daily-driver site usability.

They share the daemon, protocol, native backend, live sessions, and hand-off.
They do not depend on each other's product policy. Reusable behavior is
promoted into the shared platform with a capability contract and conformance
coverage; it is never copied periodically from one product into the other. The
portfolio and track plans live in the [roadmap index](docs/roadmap.md).

## What stays portable

The target architecture separates product behavior from native mechanisms.
Names below describe responsibilities, not a commitment to one large refactor.
Extraction should follow working seams and preserve the wire format
throughout.

| Layer | Portable responsibility | Native responsibility |
| --- | --- | --- |
| `hwatu-ipc` | request/response/event schema, capability model, compatibility tests | local endpoint naming and security |
| client (`hwatu`) | argument parsing, MCP translation, request framing, structured output | connect/spawn/update/install integration |
| daemon core | IDs, pools, profiles, quotas, request dispatch, session policy, verification workflows | main-loop scheduling and process lifecycle |
| verification core | image diff, regions, tolerances, assertions, JSON shaping | capture pixels and engine events |
| browser backend | backend-neutral view/window capabilities and lifecycle contracts | WebKitGTK, WKWebView, or WebView2 implementation |
| platform shell | intent such as focus, background, open download, paths, notifications | GTK/Wayland/X11, AppKit, or Win32 behavior |

The backend contract should be capability-oriented rather than a giant trait
that pretends every engine is identical. At minimum it must make these
operations explicit:

- create, prewarm, navigate, close, focus, unfocus, and resize a view;
- evaluate JavaScript and bridge structured messages;
- observe load, console, network, download, permission, popup, and crash
  events;
- capture viewport and full-document pixels;
- inject scripts and content-blocking rules before document code;
- persist or truthfully report inability to persist session state; and
- report engine name/version plus supported and degraded capabilities.

Unsupported behavior returns a stable structured error. It never silently
succeeds, shells out to a different browser, or changes meaning by platform.

Session persistence deserves emphasis: granted sessions are the product of a
human hand-off and must survive daemon restarts, be enumerable, and be
revocable. A backend that cannot persist a session says so in its capability
matrix; it does not pretend by re-authenticating silently.

## Native platform targets

### Linux

WebKitGTK + GTK remains the reference implementation while portable seams are
extracted. Wayland/X11 details such as `app_id`, display availability, and
window mapping stay inside the Linux shell. Existing behavior and performance
are regression constraints, not disposable prototype details.

### macOS

WKWebView runs inside an AppKit host. The daemon is a per-user process, the
local transport uses a protected Unix-domain socket, and
caches/configuration use Apple-standard directories. Focus/unfocus must
operate on the same `WKWebView` and avoid activating the app for agent-only
work. Distribution is a signed and notarized universal application/CLI
package.

Feasibility is measured, not assumed. Probes in
[`scripts/probes/macos/`](scripts/probes/macos/) establish these constraints;
the analysis and competitive reasoning are in
[`docs/research-macos-headless.md`](docs/research-macos-headless.md).

- An unbundled CLI binary with `NSApplication.activationPolicy = .prohibited`
  can load, evaluate, and snapshot an offscreen `WKWebView` with no dock icon
  and no focus steal. Warm eval is under 1 ms and warm snapshot is 2 ms, so
  the warm-service architecture transfers intact.
- **`requestAnimationFrame` is throttled to 0-1 fps whenever the view is
  offscreen or occluded**, and no window-level trick recovers it. macOS
  therefore reports `motion.realtime` as degraded. Readiness must never
  depend on frame counts. Deterministic WAAPI seek does render the sought
  instant offscreen, so the verification instrument keeps working; realtime
  motion and scroll-feel work only mean anything in a visible window.
- `takeSnapshot` is viewport-only. Full-document capture has no direct peer
  for WebKitGTK's `SnapshotRegion::FullDocument` and needs an explicit
  strategy.
- A logged-in GUI session appears to be required. Hosting `WKWebView` on a
  GUI-less macOS CI runner is unproven and must be verified or documented as
  unsupported rather than implied.

### Windows

WebView2 runs inside a Win32 host using the installed Evergreen runtime. The
local transport is a per-user named pipe with an ACL restricted to that user.
Headless/background work uses a real hidden or non-activating host window
whose same WebView2 controller can be shown for hand-off. Distribution
provides signed x86_64 and arm64 artifacts and diagnoses a missing WebView2
runtime clearly.

## Portability rules

1. Platform dependencies live behind target-specific crates or modules and
   target-specific Cargo dependency tables.
2. Portable crates compile and run their unit/contract tests on all three CI
   runners without native browser SDKs.
3. Paths come from platform directory APIs. No portable module assumes
   `/tmp`, `$HOME`, XDG variables, a Unix UID, or slash-separated paths.
4. Transport is an interface. Unix sockets and Windows named pipes carry the
   same framing, timeout, subscription, and authentication semantics.
5. Daemon discovery/spawn is race-safe and idempotent. Concurrent clients
   must converge on one healthy per-user daemon.
6. The public protocol evolves additively. Capabilities are negotiated before
   a caller depends on optional engine behavior.
7. Automation semantics are tested against shared fixtures. Backend-specific
   code does not fork the CLI or MCP surface.
8. Performance is measured per platform. Linux numbers are not marketed as
   Windows or macOS numbers.

## Known hard problems

Portability work should attack these risks early rather than discovering them
after three backends exist.

- **Three UI loops.** GLib, AppKit, and Win32/COM have different thread and
  reentrancy rules. The daemon core needs a small `spawn_on_ui`/timer
  contract, while engine objects remain confined to their native UI thread.
- **Prewarm may not transfer.** A native engine can make view creation or
  process reuse opaque. Each backend must prove warm adoption with
  measurements and expose degraded pooling honestly if the OS prevents it.
- **Pixels have provenance.** Device scale, color profile, font
  rasterization, scrollbar policy, and capture APIs differ. Screenshot
  metadata and baseline keys must include platform, engine/version, scale,
  viewport, and color space.
- **Content blocking is asymmetric.** WebKit content-rule lists and WebView2
  request interception do not have identical coverage or cost. The
  capability matrix must distinguish native blocking levels rather than
  promise one implementation.
- **GUI CI can lie.** A compile-only runner cannot prove non-activation,
  focus hand-off, DPI behavior, or GPU capture. Hosted CI covers contracts,
  while a small physical/native runner pool owns those release gates and
  publishes its evidence.
- **Sites gate on accumulated identity.** Some services recognize a browser
  profile with years of history and challenge a fresh one harder. Delegated
  hand-off plus persistent granted sessions is the answer for almost all of
  these; the residue must be documented honestly rather than papered over
  with fingerprint tricks that violate the constitution.

## Delivery sequence and exit gates

### M0: freeze the contract and establish the matrix

- Record golden JSON fixtures for every request, response, event, and
  structured error.
- Add platform/engine metadata and capability negotiation without breaking an
  older client.
- Create Linux, macOS, and Windows CI lanes for portable crates.
- Inventory all GTK/WebKitGTK, GLib, Wayland/X11, Unix socket, XDG,
  shell-tool, installer, and updater assumptions.

**Exit gate:** protocol compatibility tests and portable crate tests pass on
all three hosted runners. Every platform assumption has an owner and
destination.

### M1: extract without changing Linux

- Isolate local transport, platform directories, daemon lifecycle,
  verification math, and engine/window operations behind narrow boundaries.
- Move GTK/WebKitGTK types out of portable orchestration.
- Keep Linux as a continuously working reference, including prewarm, check
  pool, events, focus/unfocus, screenshots, console capture, downloads, and
  recovery.

**Exit gate:** the existing Linux end-to-end suite and benchmarks pass,
portable modules contain no Linux UI/engine imports, and the CLI can exercise
an in-memory fake backend for contract tests.

### M2: prove one vertical slice on macOS and Windows

Implement the smallest complete path on each native backend:
`doctor → daemon discovery → open headless → wait → eval → screenshot →
focus → unfocus → close`.

**Exit gate:** that path runs on physical or hosted native runners, uses the
same CLI commands and JSON assertions as Linux, does not steal focus before
`focus`, and keeps the same page identity and session state through hand-off.

On macOS the slice is scoped to the verification instrument first: a warm,
focusless `check`/`diff` against a baseline, shipped without the browser
shell. That is the capability no competitor offers, and it is the part of the
codebase that touches native types least. The tiling-WM browser is explicitly
not a macOS target: there is no tiling WM to serve, and offscreen rAF
throttling makes the scroll-feel work meaningless there. See
[`docs/research-macos-headless.md`](docs/research-macos-headless.md).

### M3: reach agent-loop parity

Add snapshot/interactables, click/type/scroll/upload, expect, diff/check,
console/network errors, events/subscriptions, profiles, downloads, prompts,
popups, crash recovery, prefetch, and pooled reuse according to the
advertised capability matrix. Delegated authentication reaches parity here
too: challenge detection, hand-off, resume, and persistent granted sessions
behave identically on every platform that claims them.

**Exit gate:** the shared conformance suite passes for every capability
marked supported. Failures for unavailable capabilities are structured and
tested. Each platform publishes cold start, warm connect, open, and
complete-check latency plus idle and per-view memory.

### M4: make installation native

- Linux keeps distro packages and explicit WebKitGTK diagnostics.
- macOS ships signed/notarized universal artifacts and launch integration.
- Windows ships signed x86_64/arm64 artifacts, named-pipe lifecycle
  integration, and WebView2 runtime diagnostics.
- `setup`, `doctor`, `update`, and uninstall are previewable, idempotent, and
  reversible on every platform.

**Exit gate:** a clean OS image can install, connect an MCP client, run the
shared smoke fixture, update, and uninstall without source tools or
undocumented steps.

### M5: declare support

**Exit gate:** two consecutive releases satisfy the conformance, focus-safety,
lifecycle, soak, packaging, and performance gates on all supported
architectures. Only then may README and release metadata claim that platform
as supported.

## Swarm execution model

Portability is a dependency graph, not three agents independently rewriting
the daemon. Use long-lived ownership lanes with short integration slices.

| Lane | Owns | Must not own |
| --- | --- | --- |
| Contract | `hwatu-ipc`, capabilities, golden fixtures, compatibility policy | native backend behavior |
| Core extraction | daemon orchestration, pools, verification interfaces, fake backend | platform implementations |
| Transport/lifecycle | endpoint abstraction, discovery, spawn, shutdown, per-user security | protocol payloads |
| Linux guardian | reference backend and regression/performance gates | portable API changes made only for Linux convenience |
| macOS backend | WKWebView/AppKit implementation, packaging evidence | shared semantics |
| Windows backend | WebView2/Win32 implementation, packaging evidence | shared semantics |
| Conformance | shared fixtures, focus-safety tests, soak and fault injection | waiving failures for a platform |
| Release | signing, artifacts, clean-image install/update/uninstall | declaring parity without conformance evidence |

### How the swarm works

1. **Contract first.** The contract and conformance lanes publish a failing
   test and capability expectation before a native feature is assigned.
2. **Vertical slices.** Platform agents take one end-to-end operation at a
   time, not whole directories. A slice includes implementation, fixture
   evidence, failure semantics, and diagnostics.
3. **One writer per boundary.** The contract, core interface, and each
   backend have a single active owner. Other agents propose changes through
   tests or a small interface request to prevent merge-conflict architecture.
4. **Linux remains green.** Extraction lands only when the Linux guardian can
   demonstrate unchanged behavior and compare benchmark deltas.
5. **Two-sided gates.** A backend implementer cannot approve its own
   conformance or focus-safety gate. The verification lane reproduces it on
   the target OS.
6. **Capability honesty.** Missing parity becomes an explicit matrix entry
   and a runnable test, never a hidden conditional or documentation footnote.
7. **Small integration cadence.** Merge protocol/core changes first, then
   rebase all platform slices. Avoid long-lived platform mega-branches.
8. **Evidence is the hand-off.** Every completed swarm task reports commands,
   runner/OS and engine versions, fixture output, performance delta,
   remaining capability gaps, and files it intentionally did not inspect.

A practical first swarm wave is:

```text
Contract ─────┬─> transport/lifecycle ─┬─> macOS vertical slice
              │                        └─> Windows vertical slice
              ├─> fake backend + core extraction
              └─> conformance harness ─┬─> Linux regression gate
                                      ├─> macOS verification gate
                                      └─> Windows verification gate
```

No platform lane should begin broad feature parity until M1's fake backend
and Linux regression gate make the portable boundary executable.

## What we will not do

- Bundle Electron, CEF, or a private Chromium to manufacture uniformity.
- Ride inside the user's daily browser as an extension. Sharing the human's
  focus, tab strip, and full logged-in identity is an architecture whose
  failure modes we have measured, not a feature.
- Store, transmit, or ever observe the user's credentials. Authentication is
  delegation through hand-off; the agent keeps granted sessions, never
  secrets.
- Spoof fingerprints, solve CAPTCHAs, inject challenge tokens, or bypass
  access controls. Hwatu's identity story is honest or it does not exist.
- Add a cloud relay between the agent and the page.
- Replace the stable protocol with platform-specific CLIs or MCP tools.
- Promise pixel-identical rendering across WebKitGTK, WKWebView, and
  WebView2.
- Reduce hand-off to opening the URL in another browser.
- Put browser-shell policy into verification or portable runtime code. Tabs,
  sync, a password store, and a general extension platform remain out of
  scope. Focused daily-browser work belongs to the tiling-WM browser roadmap.
- Block useful Linux roadmap work while portability foundations are
  incremental.
- Claim support from cross-compilation alone.

## Decision test

A change belongs in Hwatu when it makes the same agent verification, delegated
identity, or live hand-off outcome reliable without weakening the constitution
above. If it only makes code look abstract, moves platform details into
conditionals, exposes a credential to make a login convenient, or buys nominal
parity by losing warmth, invisibility, measurement, or session continuity, it
is not progress.
