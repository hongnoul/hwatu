# Architecture audit (2026-09-29)

Code-only audit of the apex chain (hana client → hwatu-ipc → hwatud core →
engine layer), produced by a 4-node swarm audit + interop aggregation.
Docs were deliberately ignored; every claim below is grounded in source.

## Issue register

### 1. Untyped responses, no metadata (protocol) — NOT in current fix scope
`Response` is 2 variants: `ok { window?, windows?, adblock?, value?, path?, data? }`
and `err { message }`. All 48 request kinds funnel through the same optional
fields. No request-id correlation (order-only matching), empty `ok{}` is legal
for anything, no protocol version negotiation anywhere in the handshake.

### 2. Duplicated semantics / drift (protocol) — NOT in scope
Client and daemon each parse/validate independently: 6 duplicate enum parsers
(Viewport, LoadStage, ContentFormat, LoginFill, PressKey, ClockAction),
2 duplicate validators (batch, try_until). Already drifted: Batch allowlist is
14 daemon-side but its error string lists 12; TryUntil allows 3; `uses_eval`
contradicts `validate_batch` on nested Batch.

### 3. Base64 inline data plane — NOT in scope
Screenshots/uploads/renders travel as base64 inside single NDJSON lines
(32MiB frame cap, 16MiB inline, 8MiB render), whole-in-memory both ends,
~33% overhead, no streaming/progress/dedup.

### 4. Engine coupling smeared across modules — IN SCOPE (root target)
23 daemon modules import gtk/webkit6 directly. 7 are legitimately
presentation (window 148 sites, bar 28, main 23, notify 7, theme 4, keys 4,
launcher 2). 16 are miswired core modules totaling ~134 call sites:
automation 31, clock 15, trusted_input 13, verify 10, ipc_server 10,
console 7, prompts 7, downloads 6, mediashim/hints/focusshield/blurshield/
adblock 5 each, siteua 4, net 3, events 3. No single split node exists;
the engine boundary is a jagged coastline.
16 modules are already fully portable (zero engine tokens): palette,
compositor, snapdiff, snapbudget, session, search, sitedata, share,
private_files, passfill, observe, history, external, darkmode, abp,
coverage (Reply-indirect only).

### 5. Daemon ownership fusion — IN SCOPE (forced prerequisite)
One `Daemon` struct owns gtk::Application (app lifecycle), the prewarmed
WebView pool, and NetworkSessions, single-threaded Rc/RefCell on the GTK
main loop. Engine cannot exist without GTK init. Pool + network ownership
must move core-side; app lifecycle stays shell-side.

### 6. Fake-display headless — IN SCOPE (falls out of 4+5)
Headless = spawn a managed child compositor when WAYLAND_DISPLAY/DISPLAY
absent, push HWATU_HEADLESS_SIZE viewports, patch the dpr leak (GTK inherits
compositor fractional scale onto headless surfaces), shield focus. All are
artifacts of simulating a display headless never needed. Deletable once a
display-free backend satisfies the trait.

### 7. Dual-ownership client/daemon logic — NOT in scope
Logic split from the state it owns: resolve_path/cwd absolutization
client-side, baseline-dir coherence + RENDER_MAX client-only, agent-mode
env scan client-side, expect_watch 2-connection sniff race, CLI-vs-MCP
default/clamp divergence, MCP covers ~31/48 requests.

Edge accounting: 30 miswired edges total = 16 (issue 4, need trait node)
+ 12 (issue 7/2, fixable by deleting duplicate owners) + 2 (issue 5, node
split). ~24 of ~50 nodes fully correct as-is.

## Confirmed fix plan (this effort): EngineBackend trait

Solves 4 directly, forces 5, deletes 6 as a corollary. Does not touch
1/2/3/7 (separate protocol track, parallelizable, no shared edges).

Sequence (each step leaves tree green, monotonically reduces
boundary-crossing edges):

- **P2a — portable vocabulary.** Engine-free KeyCode/modifier enums,
  load-stage/signal enums, snapshot region types in core. Kills
  gdk::Key/LoadEvent/SnapshotRegion leakage from core signatures.
- **P2b — trait + reference impl.** `EngineBackend` (create view, navigate,
  eval, user scripts/messages, content filters, snapshot, input inject,
  scale/viewport, session/scheme) + WebKitGTK implementation. Parity proven
  against existing tests before any caller migrates.
- **P2c — migrate the 16 miswired modules** onto the trait (134 call sites).
- **P2d — ownership split.** Pool + NetworkSessions to core; gtk::Application
  + window/bar/keys/theme/notify/launcher become the Linux visible shell.
- **P2e — headless via trait.** Prove headless CI parity through the trait,
  then delete compositor-spawn/dpr/focus-shield workarounds.

Trait discipline (anti-god-interface guards):
1. Every method must be implementable by ≥2 real backends (WebKitGTK and a
   hypothetical WPE-headless). GTK-only capability belongs in the shell.
2. No GTK vocabulary (surfaces, widgets, GdkEvents, focus) in signatures.
3. Capability-shaped, not API-mirror-shaped: "pixels at scale X",
   "deliver input", not "get_surface".

Deferred by design: WPE backend (becomes a swap, not a rewrite, once P2
lands), macOS/Windows shells, protocol track P0/P1.
