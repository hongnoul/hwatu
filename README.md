<div align="center">

# hwatu

[![Latest Release](https://badgen.net/github/release/hongnoul/hwatu?icon=github)](https://github.com/hongnoul/hwatu/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](LICENSE)
[![CI](https://github.com/hongnoul/hwatu/actions/workflows/ci.yml/badge.svg)](https://github.com/hongnoul/hwatu/actions/workflows/ci.yml)

**The verification browser for coding agents.**

</div>

- **STOP your agent claiming "pixel-perfect." Make it prove 97.49%.**
- **STOP browser windows stealing your focus. Headless by default, you keep typing.**
- **STOP shipping 170 MB of Chromium. One static binary + your distro's webkitgtk.**

![hwatu spawning windows in ~48ms from a warm daemon](docs/assets/spawn-demo.svg)

## Documents

- [Agent guide](docs/agents.md) — protocol, primitives, verification loops
- [Human guide](docs/human.md) — the tiling-WM browser side
- [Benchmarks](docs/benchmarks.md) — every number, measured, with methodology
- [Roadmap](docs/roadmap.md) — plan of record, priorities, non-goals

## Quick Start

```bash
curl -fsSL https://raw.githubusercontent.com/hongnoul/hwatu/main/scripts/install.sh | bash
```

One static binary plus your distro's `webkitgtk-6.0` (the installer
checks). On Arch: `yay -S hwatu`. From source: `cargo build --release`.

The installer installs the binaries but does not modify agent configuration.
Register hwatu with Claude Code explicitly (the default local scope applies
only to the current project):

```sh
claude mcp add hwatu -- hwatu mcp
```

For a config shared with the repository, use `--scope project`, then start
`claude` once to approve the new project-scoped server. Other MCP clients
(Cursor, ...) use the equivalent entry below; jcode drives hwatu natively:

```json
{ "mcpServers": { "hwatu": { "command": "hwatu", "args": ["mcp"] } } }
```

Or skip MCP entirely: every command is a short CLI call or one
newline-delimited JSON line over a Unix socket.

```sh
hwatu localhost:3000       # open a window like you open a terminal
```

## Features

- [x] Pixel-diff scoring: match percent + diff regions + heatmap (`diff`)
- [x] Animations as numbers: duration, easing, velocity (`motion`)
- [x] Deterministic animation frames: pin all animations at time t (`seek`)
- [x] Page state as JSON, tokens not pixels (`snapshot`)
- [x] Real input events with structured errors (`click` / `type` / `scroll` / `upload`)
- [x] JS errors, console output, failed requests (`console`)
- [x] One-call page assertions with polling (`expect`)
- [x] Headless / background / focused as a *per-window* property, switchable live
- [x] Human hand-off: `hwatu focus <id>` drops the live session into your tiling WM
- [x] CAPTCHA / anti-bot detection with structured wait/resume (`challenge`)
- [x] MCP server, plain CLI, and a 1-line JSON socket protocol
- [x] A minimal WebKit browser for humans: native ad blocking, vim-style bar, crash restore

## Why not Playwright or chrome-devtools-mcp?

Playwright and chrome-devtools-mcp are, at their core, automation
APIs: they let an agent *drive* a browser, then hand back raw
screenshots and DOM for the agent to eyeball.

hwatu is different. It is a *verification* browser: the measurement
primitives are built in, and the browser itself is a warm daemon
where a window costs 13 ms and headless is a window property, not a
launch mode.

That is why the loop looks like this, real commands, real output:

```sh
hwatu --headless localhost:3000        # its window; you never see it
hwatu --headless staging.example.com   # the reference

hwatu diff --id 2 --other 1 --heatmap /tmp/heat.png
# {"match_percent":85.13,"regions":[{"x":0,"y":160,"w":2048,...}]}

hwatu motion --id 1                    # the reference's animations, as numbers
# easing cubic-bezier(0.25,1,0.5,1), 300ms, marquee 29.78px/s ...

# ...agent edits code...

hwatu diff --id 2 --other 1
# {"match_percent":97.49}              # climbing beats guessing
```

We ran this loop against a clone of stripe.com's landing page: an
agent took it from **85.1% to 98.8% pixel match**. Reproduce it:
[scripts/demo/](scripts/demo/). A full verification pass (open, load,
eval, screenshot, close) is **87 ms median**
([benchmarks](docs/benchmarks.md)).

And when the agent hits a CAPTCHA or a judgment call, `hwatu focus`
materializes its live session, cookies and state intact, in your
tiling WM. You act for ten seconds. It takes back over. No other
tool can do this, because everywhere else headless is decided at
launch.

## How hwatu compares

**Legend:** ✅ Yes / built-in  ·  🟡 Partial / limited  ·  ❌ No

| Capability | Playwright | chrome-devtools-mcp | hwatu |
| --- | :---: | :---: | :---: |
| Pixel-diff score + regions + heatmap | 🟡 1 | ❌ | ✅ |
| Animations as numbers, pinned mid-flight | ❌ 2 | 🟡 3 | ✅ |
| Headless ↔ headed on a *live* session | ❌ | ❌ | ✅ |
| Human hand-off mid-session, state intact | ❌ | ❌ | ✅ |
| No focus stealing at N parallel agents | 🟡 4 | 🟡 4 | ✅ |
| CAPTCHA detection + structured wait/resume | ❌ | ❌ | ✅ |
| No Node, no per-version browser download | ❌ | ❌ | ✅ |

1 `toHaveScreenshot` compares against stored goldens: pass/fail for
test suites, not a score an agent can climb.

2 Standard practice is to disable animations or fast-forward to the
end state to avoid flakes.

3 Raw CDP can query animation state, but there is no numeric
summary of easing/velocity/keyframes.

4 Fine headless; every headed window pops and takes focus.

> Comparison reflects each project at the time of writing;
> corrections are welcome. Honest caveats: raw warm latency vs
> Playwright is a tie (83 vs 82 ms), hwatu renders WebKit not
> Chromium (keep a Playwright matrix in CI for engine-specific
> bugs), and it is Linux-only today. Full head-to-head data:
> [docs/benchmarks.md](docs/benchmarks.md).

---

MIT licensed. Linux. WebKitGTK 6.
