# Hwatu vision

Status: direction of travel. The current plan of record remains
[`docs/roadmap.md`](docs/roadmap.md). This document defines the product
principles that roadmap items and platform work must preserve.

## The outcome

Hwatu should give an AI coding agent the same fast visual-verification loop on
Linux, macOS, and Windows:

1. connect to one warm local daemon,
2. open or adopt a prewarmed native web view without taking focus,
3. inspect, drive, measure, and compare the rendered page through the same JSON
   protocol,
4. materialize that exact live session when a person is needed, and
5. return it to the background without losing state.

"Cross-platform" does not mean that the Rust workspace happens to compile on
three targets. It means a user can install Hwatu in the normal way for their OS,
run `hwatu doctor`, connect an agent through CLI or MCP, complete the same
verification fixture, and perform a live human hand-off.

## Product constitution

These decisions are more durable than any backend.

- **Agent-first.** Hwatu is a visual-verification browser for coding agents, not
  a general-purpose human browser.
- **The human side earns its keep, then some.** Since v0.7.0 the human side is a
  credible primary browser for tiling WMs: mainstream keybinds, media-correct
  playback, unified shortform controls, Chromium-curve scrolling. This is not a
  pivot. Human-side quality is in scope exactly where it shares machinery with
  agent verification (the same engine that renders a page correctly for a
  person is the one an agent measures) and where it serves hand-off. The
  daily-driver feature list (tabs, sync, password fill, extensions) stays out.
- **Warm by default.** The daemon, engine, rendering context, and one useful view
  remain warm between short-lived clients.
- **Invisible until needed.** Headless and background work must not steal focus.
  Visibility is a live window property, not a process launch mode.
- **The live session is the hand-off.** Focus and unfocus preserve page, cookies,
  navigation, and in-progress work. A screenshot of a replacement browser is not
  hand-off.
- **One protocol.** CLI, MCP, and native integrations remain adapters over the
  versioned newline-delimited JSON contract in `hwatu-ipc`.
- **Native engines, portable behavior.** Use the OS engine and window stack:
  WebKitGTK on Linux, WKWebView on macOS, and WebView2 on Windows. Do not hide a
  bundled Chromium download behind the word native.
- **Measured parity, not claimed parity.** Web engines differ. Hwatu guarantees
  command semantics and reports engine/platform capabilities explicitly. Pixel
  output is only compared against a baseline captured for the same declared
  engine unless the caller opts into another policy.
- **Small core, hard boundaries.** Portable orchestration and verification math
  must not import GTK, GLib, Cocoa, Win32, or engine-specific types.

## What stays portable

The target architecture separates product behavior from native mechanisms.
Names below describe responsibilities, not a commitment to one large refactor.
Extraction should follow working seams and preserve the wire format throughout.

| Layer | Portable responsibility | Native responsibility |
| --- | --- | --- |
| `hwatu-ipc` | request/response/event schema, capability model, compatibility tests | local endpoint naming and security |
| client (`hwatu`) | argument parsing, MCP translation, request framing, structured output | connect/spawn/update/install integration |
| daemon core | IDs, pools, profiles, quotas, request dispatch, session policy, verification workflows | main-loop scheduling and process lifecycle |
| verification core | image diff, regions, tolerances, assertions, JSON shaping | capture pixels and engine events |
| browser backend | backend-neutral view/window capabilities and lifecycle contracts | WebKitGTK, WKWebView, or WebView2 implementation |
| platform shell | intent such as focus, background, open download, paths, notifications | GTK/Wayland/X11, AppKit, or Win32 behavior |

The backend contract should be capability-oriented rather than a giant trait that
pretends every engine is identical. At minimum it must make these operations
explicit:

- create, prewarm, navigate, close, focus, unfocus, and resize a view;
- evaluate JavaScript and bridge structured messages;
- observe load, console, network, download, permission, popup, and crash events;
- capture viewport and full-document pixels;
- inject scripts and content-blocking rules before document code;
- persist or truthfully report inability to persist session state; and
- report engine name/version plus supported and degraded capabilities.

Unsupported behavior returns a stable structured error. It never silently
succeeds, shells out to a different browser, or changes meaning by platform.

## Native platform targets

### Linux

WebKitGTK + GTK remains the reference implementation while portable seams are
extracted. Wayland/X11 details such as `app_id`, display availability, and
window mapping stay inside the Linux shell. Existing behavior and performance
are regression constraints, not disposable prototype details.

### macOS

WKWebView runs inside an AppKit host. The daemon is a per-user process, the local
transport uses a protected Unix-domain socket, and caches/configuration use
Apple-standard directories. Focus/unfocus must operate on the same `WKWebView`
and avoid activating the app for agent-only work. Distribution is a signed and
notarized universal application/CLI package.

### Windows

WebView2 runs inside a Win32 host using the installed Evergreen runtime. The
local transport is a per-user named pipe with an ACL restricted to that user.
Headless/background work uses a real hidden or non-activating host window whose
same WebView2 controller can be shown for hand-off. Distribution provides signed
x86_64 and arm64 artifacts and diagnoses a missing WebView2 runtime clearly.

## Portability rules

1. Platform dependencies live behind target-specific crates or modules and
   target-specific Cargo dependency tables.
2. Portable crates compile and run their unit/contract tests on all three CI
   runners without native browser SDKs.
3. Paths come from platform directory APIs. No portable module assumes `/tmp`,
   `$HOME`, XDG variables, a Unix UID, or slash-separated paths.
4. Transport is an interface. Unix sockets and Windows named pipes carry the
   same framing, timeout, subscription, and authentication semantics.
5. Daemon discovery/spawn is race-safe and idempotent. Concurrent clients must
   converge on one healthy per-user daemon.
6. The public protocol evolves additively. Capabilities are negotiated before a
   caller depends on optional engine behavior.
7. Automation semantics are tested against shared fixtures. Backend-specific
   code does not fork the CLI or MCP surface.
8. Performance is measured per platform. Linux numbers are not marketed as
   Windows or macOS numbers.

## Known hard problems

Portability work should attack these risks early rather than discovering them
after three backends exist.

- **Three UI loops.** GLib, AppKit, and Win32/COM have different thread and
  reentrancy rules. The daemon core needs a small `spawn_on_ui`/timer contract,
  while engine objects remain confined to their native UI thread.
- **Prewarm may not transfer.** A native engine can make view creation or
  process reuse opaque. Each backend must prove warm adoption with measurements
  and expose degraded pooling honestly if the OS prevents it.
- **Pixels have provenance.** Device scale, color profile, font rasterization,
  scrollbar policy, and capture APIs differ. Screenshot metadata and baseline
  keys must include platform, engine/version, scale, viewport, and color space.
- **Content blocking is asymmetric.** WebKit content-rule lists and WebView2
  request interception do not have identical coverage or cost. The capability
  matrix must distinguish native blocking levels rather than promise one
  implementation.
- **GUI CI can lie.** A compile-only runner cannot prove non-activation, focus
  hand-off, DPI behavior, or GPU capture. Hosted CI covers contracts, while a
  small physical/native runner pool owns those release gates and publishes its
  evidence.

## Delivery sequence and exit gates

### M0: freeze the contract and establish the matrix

- Record golden JSON fixtures for every request, response, event, and structured
  error.
- Add platform/engine metadata and capability negotiation without breaking an
  older client.
- Create Linux, macOS, and Windows CI lanes for portable crates.
- Inventory all GTK/WebKitGTK, GLib, Wayland/X11, Unix socket, XDG, shell-tool,
  installer, and updater assumptions.

**Exit gate:** protocol compatibility tests and portable crate tests pass on all
three hosted runners. Every platform assumption has an owner and destination.

### M1: extract without changing Linux

- Isolate local transport, platform directories, daemon lifecycle, verification
  math, and engine/window operations behind narrow boundaries.
- Move GTK/WebKitGTK types out of portable orchestration.
- Keep Linux as a continuously working reference, including prewarm, check pool,
  events, focus/unfocus, screenshots, console capture, downloads, and recovery.

**Exit gate:** the existing Linux end-to-end suite and benchmarks pass, portable
modules contain no Linux UI/engine imports, and the CLI can exercise an in-memory
fake backend for contract tests.

### M2: prove one vertical slice on macOS and Windows

Implement the smallest complete path on each native backend:
`doctor → daemon discovery → open headless → wait → eval → screenshot → focus →
unfocus → close`.

**Exit gate:** that path runs on physical or hosted native runners, uses the same
CLI commands and JSON assertions as Linux, does not steal focus before `focus`,
and keeps the same page identity and session state through hand-off.

### M3: reach agent-loop parity

Add snapshot/interactables, click/type/scroll/upload, expect, diff/check,
console/network errors, events/subscriptions, profiles, downloads, prompts,
popups, crash recovery, prefetch, and pooled reuse according to the advertised
capability matrix.

**Exit gate:** the shared conformance suite passes for every capability marked
supported. Failures for unavailable capabilities are structured and tested.
Each platform publishes cold start, warm connect, open, and complete-check
latency plus idle and per-view memory.

### M4: make installation native

- Linux keeps distro packages and explicit WebKitGTK diagnostics.
- macOS ships signed/notarized universal artifacts and launch integration.
- Windows ships signed x86_64/arm64 artifacts, named-pipe lifecycle integration,
  and WebView2 runtime diagnostics.
- `setup`, `doctor`, `update`, and uninstall are previewable, idempotent, and
  reversible on every platform.

**Exit gate:** a clean OS image can install, connect an MCP client, run the shared
smoke fixture, update, and uninstall without source tools or undocumented steps.

### M5: declare support

**Exit gate:** two consecutive releases satisfy the conformance, focus-safety,
lifecycle, soak, packaging, and performance gates on all supported architectures.
Only then may README and release metadata claim that platform as supported.

## Swarm execution model

Portability is a dependency graph, not three agents independently rewriting the
daemon. Use long-lived ownership lanes with short integration slices.

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

1. **Contract first.** The contract and conformance lanes publish a failing test
   and capability expectation before a native feature is assigned.
2. **Vertical slices.** Platform agents take one end-to-end operation at a time,
   not whole directories. A slice includes implementation, fixture evidence,
   failure semantics, and diagnostics.
3. **One writer per boundary.** The contract, core interface, and each backend
   have a single active owner. Other agents propose changes through tests or a
   small interface request to prevent merge-conflict architecture.
4. **Linux remains green.** Extraction lands only when the Linux guardian can
   demonstrate unchanged behavior and compare benchmark deltas.
5. **Two-sided gates.** A backend implementer cannot approve its own conformance
   or focus-safety gate. The verification lane reproduces it on the target OS.
6. **Capability honesty.** Missing parity becomes an explicit matrix entry and a
   runnable test, never a hidden conditional or documentation footnote.
7. **Small integration cadence.** Merge protocol/core changes first, then rebase
   all platform slices. Avoid long-lived platform mega-branches.
8. **Evidence is the hand-off.** Every completed swarm task reports commands,
   runner/OS and engine versions, fixture output, performance delta, remaining
   capability gaps, and files it intentionally did not inspect.

A practical first swarm wave is:

```text
Contract ─────┬─> transport/lifecycle ─┬─> macOS vertical slice
              │                        └─> Windows vertical slice
              ├─> fake backend + core extraction
              └─> conformance harness ─┬─> Linux regression gate
                                      ├─> macOS verification gate
                                      └─> Windows verification gate
```

No platform lane should begin broad feature parity until M1's fake backend and
Linux regression gate make the portable boundary executable.

## What we will not do

- Bundle Electron, CEF, or a private Chromium to manufacture uniformity.
- Replace the stable protocol with platform-specific CLIs or MCP tools.
- Promise pixel-identical rendering across WebKitGTK, WKWebView, and WebView2.
- Reduce hand-off to opening the URL in another browser.
- Add tabs, sync, password management, extensions, or general daily-browser UX.
- Block useful Linux roadmap work while portability foundations are incremental.
- Claim support from cross-compilation alone.

## Decision test

A portability change belongs in Hwatu when it makes the same agent verification
and live hand-off outcome reliable on another native OS without weakening the
constitution above. If it only makes code look abstract, moves platform details
into conditionals, or buys nominal parity by losing warmth, invisibility,
measurement, or session continuity, it is not progress.
