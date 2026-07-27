# Hwatu launch kit

Use this page to keep Hwatu's public story consistent. Verify performance claims
against [`benchmarks.md`](benchmarks.md) before changing numbers.

## One-line pitch

**Hwatu is a fast, interruptible verification browser for AI coding agents: one
call can load, inspect, and screenshot a real page in about 35 ms, then hand the
same live session to a human when needed.**

## Who it is for

- Developers using Claude Code, Cursor, Jcode, or another MCP-capable agent.
- Agent and harness authors who need fast rendered-page feedback.
- Linux developers who want browser automation that does not steal focus.

## Proof points

- About **35 ms** median for a warm load → eval → screenshot → close pass.
- **One tool call** for that complete verification pass.
- **13 ms** median warm window spawn.
- Live headless ↔ headed hand-off with page and session state intact.
- One Rust client and daemon, using the system WebKitGTK engine.
- Reproducible Stripe clone result: **85.1% → 98.8% pixel match**.

All measurements are Linux/WebKitGTK results on the machine documented in
[`benchmarks.md`](benchmarks.md). Hwatu is Linux-only today.

## Links

- Project: https://github.com/hongnoul/hwatu
- Website: https://hongnoul.github.io/hwatu/
- Demo: https://github.com/hongnoul/hwatu/releases/download/readme-assets/demo-v2.mp4
- Benchmarks: https://hongnoul.github.io/hwatu/benchmarks
- Technical article: https://hongnoul.github.io/hwatu/blog/13ms-window-spawn.html

## Hacker News

**Title**

> Show HN: Hwatu – a 35ms verification browser for AI coding agents

**Post**

> I built Hwatu because visual feedback was the slowest and least reliable part
> of my coding-agent loop. Agents were spending several tool calls opening a page,
> inspecting it, taking a screenshot, and then claiming the result looked right.
>
> Hwatu is a warm WebKitGTK daemon for Linux. A complete load → eval → screenshot
> → close pass takes about 35 ms median and one tool call on my machine. It also
> treats visibility as a live window property: the agent can work without taking
> focus, then hand the exact session to a human for a CAPTCHA or judgment call and
> take it back afterward.
>
> It includes pixel-diff scores and regions, deterministic animation inspection,
> structured console/network errors, CLI, JSON socket, and MCP interfaces. It is
> AGPL-3.0 and Linux-only today. Playwright still wins in several areas, including
> cross-browser coverage and cold start, so the README includes the caveats and
> reproducible benchmarks.
>
> I would especially value feedback from people building coding-agent harnesses:
> https://github.com/hongnoul/hwatu

## Reddit

Suggested communities, after checking each community's current self-promotion
rules: `r/rust`, `r/linux`, `r/LocalLLaMA`, and agent-tool communities where the
project directly answers an existing discussion.

**Title**

> I built a warm WebKitGTK verification browser for coding agents (35ms checks, live human hand-off)

**Body**

> Hwatu keeps a minimal browser daemon warm so coding agents can verify rendered
> pages without repeatedly launching Chromium or stealing focus. On my Linux
> benchmark machine, a load + eval + screenshot + close pass is about 35 ms median
> in one call.
>
> The feature I could not find elsewhere is live hand-off: a headless session can
> become visible with its state intact when a human needs to solve a CAPTCHA or
> make a visual judgment, then return to the background.
>
> It is written in Rust, uses system WebKitGTK, supports CLI/JSON/MCP, and includes
> built-in pixel diffs, animation measurements, assertions, console capture, and
> structured interaction errors. AGPL-3.0, currently Linux-only.
>
> Demo, measurements, caveats, and source:
> https://github.com/hongnoul/hwatu

## LinkedIn

> Coding agents can write UI quickly. Proving that UI is correct is still slow.
>
> I built **Hwatu**, an open-source verification browser designed around the agent
> feedback loop rather than human browsing.
>
> • one-call load → inspect → screenshot in ~35 ms median  
> • built-in pixel-diff scoring and animation measurement  
> • invisible by default, with no focus stealing  
> • live hand-off of the same session from agent to human and back  
> • CLI, JSON socket, and MCP interfaces
>
> It is a Rust/WebKitGTK project for Linux, released under AGPL-3.0. The repository
> includes reproducible benchmarks and the cases where Playwright remains the
> better choice.
>
> Source and demo: https://github.com/hongnoul/hwatu
>
> I would love feedback from people building coding agents and visual eval loops.
>
> #opensource #rust #aiagents #developertools #linux

## X / Bluesky

**Single post**

> I built Hwatu: a verification browser for AI coding agents. One call loads,
> inspects, and screenshots a real page in ~35 ms. It stays invisible until a
> human is needed, then hands over the same live session and takes it back.
> Rust + WebKitGTK, AGPL, Linux. https://github.com/hongnoul/hwatu

**Thread outline**

1. The problem: agents can modify UI faster than they can verify it.
2. Demo clip showing headless verification and live hand-off.
3. Explain the warm daemon and the measured 35 ms pass.
4. Show pixel-match improvement from 85.1% to 98.8%.
5. State caveats clearly: Linux/WebKitGTK today; Playwright for cross-browser CI.
6. Link the repository and ask harness authors for concrete feedback.

## Directory copy

**Tagline, 60 characters**

> Fast visual verification for AI coding agents

**Short description**

> A warm, interruptible browser daemon that gives coding agents one-call visual
> checks, pixel diffs, structured page state, and live human hand-off.

**Suggested categories**

Developer Tools, AI Agents, Browser Automation, Testing, Open Source.

## Seven-day launch sequence

| Day | Action | Goal |
| --- | --- | --- |
| 0 | Confirm GitHub social preview, topics, website card, release assets | Every shared link renders clearly |
| 1 | Submit Show HN and stay available to answer technical questions | Reach agent and dev-tool builders |
| 2 | Share the demo clip on X/Bluesky and LinkedIn | Earn visual, easily reshared discovery |
| 3 | Post one technically tailored Reddit submission | Start a useful community discussion |
| 4 | Send personal notes to 5–10 relevant harness/tool maintainers | Obtain qualified feedback, not mass outreach |
| 5 | Share the 13 ms architecture article separately | Give the project a second technical entry point |
| 7 | Publish a short response to repeated questions or benchmark requests | Convert launch feedback into durable content |

Do not post identical copy everywhere at once. Answer every substantive launch
comment, record recurring objections, and update the README when a question
appears at least twice.

## Measurement

Record a baseline immediately before the first post, then check at 24 hours and
7 days:

- GitHub stars and forks.
- Unique repository visitors and referring sites from GitHub Traffic.
- Release asset downloads.
- Website visits, if privacy-respecting analytics are enabled.
- Successful `hwatu doctor` runs, only if anonymous telemetry is explicitly
  designed and opt-in. Do not add telemetry merely for launch measurement.
- Qualified signals: issues, reproducible bug reports, integrations, and repeat
  contributors.

A good first launch outcome is not a raw impression count. It is **10 qualified
users who install Hwatu, complete `doctor`/`demo`, and explain whether it improves
their agent loop**.
