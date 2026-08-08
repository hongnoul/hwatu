# Hwatu roadmap

Status: current as of 2026-08. This index is the portfolio plan of record. The
three linked roadmaps own their respective priorities and shipped history.
Documentation and marketing should match this product model.

Roadmap ordering is reviewed weekly using the evidence and scoring rules in
[continuous-improvement.md](continuous-improvement.md). Repeated user pain and
first-check activation outrank speculative feature breadth.

## Product model

Hwatu is one native browser platform with two products:

1. **AI verification:** a warm native-engine browser that lets coding agents
   drive, inspect, measure, compare, and prove rendered outcomes.
2. **Tiling-WM browser:** a keyboard-driven, window-per-page browser credible
   enough to be the human destination for live hand-off and daily use.

They share one daemon, protocol, engine session, and hand-off model. They do not
share product policy. The browser may optimize human navigation without adding
browser-shell concepts to agent verification; verification may optimize
machine-readable proof without dictating human interaction design.

When shared needs conflict, runtime correctness and compatibility come first,
then the agent verification loop, then live hand-off, then browser convenience.
A browser regression that prevents ordinary use is a correctness bug, not a
lower-priority feature request.

## Strategic advantage

Raw warm-check latency is useful but not the moat. Hwatu's structural advantage
is a real native window whose visibility can change without replacing the page:
an agent can work invisibly, ask for a person, and materialize the exact session
with its cookies, navigation, and in-progress state intact. The same runtime
avoids a Node dependency and private browser download while presenting a compact
CLI and JSON protocol.

The tiling-WM browser strengthens this loop when it becomes a place the user
already works. It also exercises long-session behavior that short verification
runs do not. In the other direction, the verification product provides an
external end-to-end harness for important browser journeys.

## Track roadmaps

| Track | Owns | Primary success signal |
| --- | --- | --- |
| [AI verification](roadmaps/verification.md) | CLI/MCP workflows, snapshots, assertions, diffing, deterministic observation, context hygiene | agents produce reproducible evidence with few calls and bounded context |
| [Tiling-WM browser](roadmaps/browser.md) | keyboard navigation, human-facing chrome, media, history integration, site usability, WM-native behavior | users can remain in Hwatu instead of switching browsers for ordinary work |
| [Shared platform](roadmaps/platform.md) | protocol, sessions, pools, native backends, transport, lifecycle, capabilities, hand-off | both products consume one compatible live-session runtime without duplicated machinery |

The platform is the only upstream. A reusable discovery in either product is
promoted into a platform capability; code is not periodically copied or pulled
from one product into the other.

## Dependency and promotion rules

```text
                         +----------------------+
                         | shared browser       |
                         | platform + protocol  |
                         +----------+-----------+
                                    |
                      +-------------+-------------+
                      |                           |
             +--------v---------+       +---------v--------+
             | AI verification | ----> | tiling-WM browser |
             +------------------+  E2E  +-------------------+
```

A product capability moves into the shared platform only when:

1. it has a second consumer or protects a shared live-session invariant;
2. its contract can be stated without agent-workflow or browser-UI policy;
3. unsupported native-engine behavior has an explicit capability or structured
   error;
4. protocol and conformance tests cover the extracted boundary; and
5. Linux behavior and measured performance remain green.

Browser-only concepts such as keymaps, history presentation, and workspace
placement do not enter verification. Verification-only concepts such as
assertion polling, evidence shaping, and context budgets do not enter the
browser shell. Both may use engine-neutral session, navigation, observation,
input, capture, and event capabilities from the platform.

## Source boundary migration

The current three-crate workspace remains valid while boundaries are extracted:

```text
crates/ipc       versioned JSON-line request/response/event contract
crates/hwatu     thin CLI and MCP adapters
crates/hwatud    current GTK/WebKit runtime and product composition root
```

Extraction follows working seams instead of a directory-only rewrite. The
target responsibility map is:

```text
crates/runtime-core       portable session, pool, dispatch, and capability policy
crates/verification       engine-neutral assertions, diff math, and evidence shaping
crates/backend-webkitgtk  Linux native view/window implementation
crates/browser-shell      tiling-WM interaction and human-facing browser policy
crates/hwatud             composition root for the selected native backend
crates/hwatu              CLI and MCP product adapters over crates/ipc
```

A target crate is created only when its existing code can compile and test
without reaching back through the old boundary. Until then, clear product
modules may remain in `hwatud`; ownership is defined by the roadmaps, not by a
premature file move. The stable wire format is preserved throughout.

## Integration cadence

- Product work may ship independently when it stays behind an existing platform
  contract.
- Shared contract changes land first and additively; product changes then consume
  them.
- Cross-track milestones name their dependency rather than duplicating the same
  item in two roadmaps.
- The verification product is the external end-to-end harness for important
  browser journeys. The browser is the long-session dogfood environment for the
  shared runtime.
- Every performance claim is measured, and every native capability is reported
  honestly rather than normalized through a hidden fallback browser.

## Shared non-goals

- Not a scraping browser. That is Lightpanda's job.
- Not a cross-browser E2E matrix. Playwright owns that category; Hwatu reports
  its actual native engine and capabilities.
- Not a CAPTCHA or anti-bot bypass tool. Challenge handling is detection and
  human hand-off only.
- No bundled Chromium disguised as a native backend.
- No tabs, sync service, general extension platform, or password store of our
  own. The tiling WM owns window grouping, and focused integrations may solve
  concrete browser needs.
