# Hwatu vision

Status: the constraint set. It changes rarely and only on purpose. The plan
of record is [`docs/roadmap.md`](docs/roadmap.md), and the cross-platform
execution plan is
[`docs/roadmaps/native-platforms.md`](docs/roadmaps/native-platforms.md).
Plans bend to this document, never the reverse.

## One sentence

Hwatu gives every agent its own invisible, real browser on the user's
machine, on Linux, macOS, and Windows alike.

## Problem

Agents need the web, and today's options make them pay for it.

- **Automation frameworks** (Playwright, CDP) are built for test suites. One
  page check costs five calls and hundreds of milliseconds.
- **Extensions in the human's browser** share the human's focus, tabs, and
  logins. Every action is one `active: true` away from stealing the screen.
  We measured a production bridge stealing focus on three of six core
  actions.
- **Cloud browser services** move the session outside the user's trust
  boundary, meter every page, and sell spoofed fingerprints as identity.

## Outcome

A user on any of the three operating systems can:

1. install Hwatu the normal way for their OS,
2. run `hwatu doctor` and see green,
3. give their agent a skill, a plain-text recipe such as "order my usual
   from DoorDash and stop before paying",

and the agent completes it. It opens pages instantly and invisibly, reads and
drives them as data, asks the human only for what a human must provide, and
keeps what it was granted so it never asks twice.

The same skill works unmodified on every OS. If a skill needs an `if macOS`
branch, Hwatu has a bug.

## Three capabilities, one session

### 1. Verification

The first product. It wins every conflict. "Pixel-perfect" is a claim.
`match_percent: 97.49` is a measurement. One call opens a page, waits, reads
it, screenshots it, and cleans up. Pixel diffs are scores an agent can climb.
Animations are numbers. Errors are data.

### 2. Granted identity

The agent hits a login wall. It asks the human for the credential or the
one-time code and enters it headlessly into the real site. The session
persists across restarts, so the human is asked once. The agent holds only
the logins it was granted, not the user's whole browsing life. What was
granted is the permission boundary.

A challenge the agent cannot relay as text is reported as a structured block,
with the session intact for whatever the human decides.

### 3. Inspection on demand

When the human wants to see what the agent sees, the live session renders on
request: a screenshot, a pixel diff, or a materialized view of the real page.
Seeing is pull, never push. The session returns to headless with nothing
lost.

Verification makes the agent trustworthy. Granted identity makes it capable
on the authenticated web. Pull-only inspection keeps the human's screen the
human's.

## Constitution

These outlive any backend.

- **Headless is the invariant, not a mode.** Every capability works without
  a visible window. A feature that requires visibility is out of scope.
- **Nothing appears unsummoned.** Visibility exists only as an explicit
  human request. It is a debugging aid, never a dependency.
- **Verification leads.** Its interfaces stay evidence-focused.
- **Credentials pass through.** The human grants a login, the agent enters
  it, the session persists. Hwatu keeps no password store of its own. It may
  read from a manager the user already runs, and it never logs or retains
  the secret.
- **Irreversible actions need a fresh human yes.** Payments, sends, and
  deletes stop for confirmation. A page can never authorize them.
- **Warm by default.** Engine and views stay warm between clients.
- **Local-first, no rent.** Everything runs on the user's machine. No relay
  sees the sessions. No metering shapes agent behavior.
- **One protocol.** The CLI, MCP, and every integration are adapters over the
  versioned JSON contract in `hwatu-ipc`. Skills target that contract, not an
  OS.
- **Native engines.** WebKitGTK on Linux, WKWebView on macOS, WebView2 on
  Windows. No bundled Chromium behind the word native.
- **Measured parity.** Engines differ. Hwatu guarantees command semantics and
  reports capabilities explicitly. Pixels compare only against baselines from
  the same declared engine unless the caller opts out.
- **Small core, hard boundaries.** Portable code never imports GTK, GLib,
  Cocoa, Win32, or engine types. Platform code exists only to host an
  invisible view, deliver input to it, and capture pixels from it.

## One product

Hwatu is the headless browser for agents. It is not also a browser for
humans. The tiling-WM experiments (bar, keybinds, scroll feel, media chrome)
proved the engine renders real pages correctly, and they are done. That code
is pruned, not maintained. Rendering a page for human eyes is an inspection
feature of the agent product, not a second product.

This is a deliberate trade. Human-browser chrome is platform-specific by
nature, none of it ports, and it grows faster than the user base with every
daily-driver ambition. Headless capability is the portable part. One product
keeps the core small and makes three platforms tractable.

## What stays portable

Product behavior separates from native mechanism.

| Layer | Portable | Native |
| --- | --- | --- |
| `hwatu-ipc` | schema, capability model, compatibility tests | endpoint naming, security |
| client (`hwatu`) | parsing, MCP translation, framing, output | connect, spawn, update, install |
| daemon core | IDs, pools, profiles, quotas, dispatch, session policy | main loop, process lifecycle |
| page scripts | snapshot ranking, element matching, content extraction | none |
| verification core | image diff, regions, tolerances, assertions | pixel capture, engine events |
| browser backend | view capability contract | WebKitGTK, WKWebView, WebView2 |

Every backend must be able to:

- create, prewarm, navigate, resize, and close an invisible view,
- evaluate JavaScript and bridge structured messages,
- observe load, console, network, download, permission, popup, and crash,
- deliver trusted input without touching the user's cursor or focus,
- capture viewport and full-document pixels,
- inject scripts and content-blocking rules before document code,
- persist granted sessions, or say plainly that it cannot,
- report engine name, version, and capability status.

Unsupported behavior returns a stable structured error. It never silently
succeeds, shells out to another browser, or changes meaning by platform.
Granted sessions survive daemon restarts, can be listed, and can be revoked.

## Portability rules

1. Platform dependencies live in target-specific crates, modules, and
   dependency tables.
2. Portable crates compile and test on all three CI runners without native
   SDKs.
3. Paths come from platform APIs. No portable module assumes `/tmp`, `$HOME`,
   XDG, a Unix UID, or slash separators.
4. Transport is an interface. Unix sockets and named pipes carry the same
   framing, timeouts, subscriptions, and authentication.
5. Daemon discovery and spawn are race-safe and idempotent. Concurrent
   clients converge on one healthy per-user daemon.
6. The protocol evolves additively. Capabilities are negotiated before use.
7. Semantics are tested against shared fixtures. Backends never fork the CLI
   or MCP surface.
8. Performance is measured per platform. Linux numbers are never marketed as
   another OS's numbers.

## Support is earned, not compiled

Cross-compiling for three targets is not cross-platform. A platform is
supported only when, from a clean OS image, a user can install Hwatu, pass
`hwatu doctor`, connect an agent, pass the shared conformance fixtures, and
run an unmodified reference skill end to end. Until then, README and release
metadata do not claim it. The gates live in
[`docs/roadmaps/native-platforms.md`](docs/roadmaps/native-platforms.md).

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

## What we will not do

- Build a browser for humans. No tabs, bar, keybinds, scroll feel, media
  chrome, themes, or reader mode.
- Bundle Electron, CEF, or a private Chromium.
- Ride inside the user's daily browser as an extension.
- Store user credentials.
- Spoof fingerprints, solve CAPTCHAs, inject tokens, or bypass access
  controls.
- Add a cloud relay between agent and page.
- Ship platform-specific CLIs, MCP tools, or skills.
- Promise pixel-identical rendering across engines.
- Block Linux work on portability foundations.
- Claim support from cross-compilation alone.

## Decision test

A change belongs in Hwatu when it makes headless verification or granted
identity more reliable, on more machines, without weakening the
constitution. If it requires a visible window, keeps a credential, moves
platform details into conditionals, makes a skill OS-aware, or buys nominal
parity by losing warmth, invisibility, or measurement, it is not progress.
