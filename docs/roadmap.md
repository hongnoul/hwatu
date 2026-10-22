# Roadmap

Status: current as of 2026-10. Scope and constraints are in
[VISION.md](../VISION.md). This page indexes the plans.

Ordering is reviewed weekly using the evidence and scoring rules in
[continuous-improvement.md](continuous-improvement.md). Repeated user pain
and first-check activation outrank feature breadth.

## Product

One product: a headless browser that runs on the user's machine and is driven
by agents through a CLI and MCP. Target platforms are Linux, macOS, and
Windows. Linux is supported today.

When priorities conflict, the order is runtime correctness and compatibility,
then the agent command loop, then structured stops and `show`.

## Plans

| Plan | Owns |
| --- | --- |
| [Verification](roadmaps/verification.md) | CLI and MCP workflows, snapshots, assertions, diffing, deterministic observation, context limits |
| [Shared platform](roadmaps/platform.md) | protocol, sessions, pools, transport, lifecycle, capabilities, `show` |
| [Native platforms](roadmaps/native-platforms.md) | backend contract, macOS and Windows hosts, packaging, support gates |

## Human involvement

The browser returns structured stops for login walls and irreversible actions
and renders the live session through `show`. CAPTCHAs and anti-bot challenges
are handled with `challenge`, `shot`, `snapshot`, `click --trusted`, and
`type`. If the agent cannot clear one, `challenge` returns a structured block
and the agent can hand off through `show`.

## Integration cadence

- Shared contract changes land first and additively. Adapters then consume
  them.
- Every performance claim is measured.
- Every native capability is reported as it is, with no hidden fallback
  browser.

## Non-goals

- Scraping at scale.
- A cross-browser test matrix. The browser reports its native engine and
  capabilities.
- A browser for humans, a skill library, stored credentials, bundled
  Chromium, fingerprint spoofing, third-party solver services, injected
  challenge tokens, or access-control bypasses.
