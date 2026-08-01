<div align="center">

# hwatu

[![Latest Release](https://badgen.net/github/release/hongnoul/hwatu?icon=github)](https://github.com/hongnoul/hwatu/releases)
[![License: AGPL-3.0](https://img.shields.io/badge/license-AGPL--3.0-blue?style=flat-square)](LICENSE)
[![CI](https://github.com/hongnoul/hwatu/actions/workflows/ci.yml/badge.svg)](https://github.com/hongnoul/hwatu/actions/workflows/ci.yml)

**Your agents are blind without hwatu**

<a href="https://github.com/hongnoul/hwatu/releases/download/readme-assets/demo-aiuc.mp4"><img src="https://github.com/hongnoul/hwatu/releases/download/readme-assets/demo-aiuc.webp" alt="An agent verifies aiuc.com with hwatu: one command returns pixel-match scores for four responsive viewports, then the live page pops into view for human hand-off" width="800"></a>

</div>

hwatu is a visual verification harness for coding agents, built as a
WebKit daemon. Instead of "looks right to me", your agent gets
**one-call verified page checks in ~35 ms**, **pixel-diff scores it
can climb**, **animations as numbers**, and **headless windows that
never steal your focus**, at any parallelism.

For human-in-the-loop tasks (e.g. Captcha), hwatu features a lightweight
visual verification frontend renderer written in WebKit and a caller
function. For tiling WMs (Hyprland, sway, **niri**, i3), hwatu is
intended to replace your primary daily browser. Our current goal is to
provide scrolling short-form content experience in mobile-level framerate.

## Documents

- [Vision](VISION.md): durable product principles, native platform strategy, swarm model
- [Agent guide](docs/agents.md): protocol, primitives, verification loops
- [Human guide](docs/human.md): daily driving hwatu in a tiling WM, keybinds, media, hand-off
- [Benchmarks](docs/benchmarks.md): every number, measured, with methodology
- [Roadmap](docs/roadmap.md): plan of record, priorities, non-goals
- [Continuous improvement](docs/continuous-improvement.md): activation metric, feedback loop, weekly cadence
- [Launch kit](docs/launch-kit.md): reusable copy, channels, and measurement plan

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/hongnoul/hwatu/main/scripts/install.sh | bash
```

One static binary plus your distro's `webkitgtk-6.0` (the installer
checks). On Arch: `yay -S hwatu`. From source: `cargo build --release`.

Then pick your door, or take both:

```sh
hwatu setup             # agent: detect Claude Code, Cursor, Jcode, or MCP
hwatu localhost:3000    # human: open a window like you open a terminal
```

## Real eyes for your coding agent

- **STOP your agent claiming "pixel-perfect." Make it prove 97.49%.**
- **STOP paying 5 tool calls per page check. `hwatu check` is one call, ~35 ms (beats warm-server Playwright ~9x).**
- **STOP browser windows stealing your focus. Headless by default, you keep typing.**
- **STOP shipping 170 MB of Chromium. One static binary + your distro's webkitgtk.**

`hwatu setup` detects supported coding agents and prints the
available connections without changing their config. Choose a client
explicitly when you are ready:

```sh
hwatu doctor
hwatu setup --client claude --scope project --dry-run
hwatu setup --client claude --scope project
hwatu demo
```

Setup is previewable, idempotent, and reversible with the same client
and scope plus `--undo`. Manual MCP configuration remains one
portable entry:

```json
{ "mcpServers": { "hwatu": { "command": "hwatu", "args": ["mcp"] } } }
```

Or skip MCP entirely: every command is a short CLI call or one
newline-delimited JSON line over a Unix socket.

Connecting hwatu makes its tools available; a project instruction
tells the agent when to use them. Add this to `AGENTS.md`,
`CLAUDE.md`, Cursor rules, or the equivalent for your harness:

```markdown
## Frontend verification

Use Hwatu after frontend changes. Exercise the affected user journey and
verify its intended visible, navigational, or persisted result with `expect`.
A successful click or clean console is not proof of success. Check `console`
for additional JavaScript and request failures after verifying the outcome.
```

Then make the task's proof concrete:

```text
Implement display-name editing on /settings. Use Hwatu to enter “Test User,”
save it, verify the visible success state, reload, confirm persistence, and
report any console errors.
```

The verification loop, real commands, real output:

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
[scripts/demo/](scripts/demo/). A second, real-agent scenario against
AIUC (four responsive viewport diffs followed by live human hand-off)
is reproducible with evidence manifests from
[scripts/demo-aiuc/](scripts/demo-aiuc/).

A full verification pass (open, load, eval, screenshot, close) is
**one command, one tool call, ~35 ms median**
([benchmarks](docs/benchmarks.md)):

```sh
hwatu check localhost:5173 --eval 'document.title' --shot=/tmp/after.png
# {"title":"My App","eval":"My App","shot":"/tmp/after.png",
#  "console":[...],"load_ms":13,"total_ms":35}
```

Generated HTML in hand and no server? `hwatu render` is the same
one-call pass with the markup as input: no temp file, no
`python3 -m http.server`:

```sh
echo '<h1>generated</h1>' | hwatu render --stdin --shot=/tmp/gen.png
# {"rendered":true,"shot":"/tmp/gen.png","load_ms":5,"total_ms":28}

# React to load, console, download, and window events without polling.
hwatu watch --kinds load,console
# {"event":"load","seq":1,"window_id":7,"data":{"state":"started",...}}
```

MCP clients can call `subscribe_events` for the same stream as
`notifications/hwatu/event`. See the full [agent guide](docs/agents.md),
including a larger copy-paste policy and verification loops.

Everywhere else, headless is decided at launch and a human can never
see the session at any price. In hwatu it's a window property,
switchable live, in both directions. And because hwatu is also the
browser you already live in, the hand-off lands in a window that
behaves like every other window on your desk, not a viewer bolted on
for emergencies.

`challenge` is detection and hand-off only, by design: no solver
APIs, no token injection, no fingerprint games.

## Agents loop, you watch some reels

The hand-off works because hwatu is also a real browser, one built
for tiling WMs. `hwatu <url>` opens a window like your terminal opens
a shell (your WM is the tab bar, there is none in the window), with
mainstream keybinds (`ctrl+l`, `ctrl+f`, `ctrl+k` palette, all
rebindable via dotfile), native ad blocking (~119k EasyList rules
compiled into WebKit's content-extension engine, zero JS in the
request path), Chromium-curve scrolling, unmuted autoplay, a
blur-shield that took Shorts from ~34 to ~95 fps, and one shortform
control scheme (arrows snap exactly one video, Space pauses, hold
ArrowRight for 2x) across Reels, Shorts, and TikTok. High framerates
help oneshotting websites with complicated scroll-anchored animation
logic (e.g. scale.com). Because of this reason, hwatu is optimized for
consuming short-form content with much less resources than what you
would have needed with Chromium or Firefox. The demo video below shows
why hwatu is an excellent alternative browser option for your system,
especially for tiling WMs:

<a href="https://github.com/hongnoul/hwatu/releases/download/readme-assets/demo-shortform.mp4"><img src="https://github.com/hongnoul/hwatu/releases/download/readme-assets/demo-shortform.webp" alt="hwatu daily driving: quarter-width window spawns, buttery Chromium-curve scrolling, and one-keypress-one-reel shortform controls on Instagram Reels" width="800"></a>

Every window shares the one warm daemon (~56 MB per extra window),
suspends when unfocused, and crash-restores at its last URL. Honest
gaps: no Widevine or passkeys in WebKitGTK, so keep a fallback bound
for Netflix. Ready-made WM configs
([hyprland](examples/hyprland.conf), [sway](examples/sway.config),
[niri](examples/niri.kdl)), the full keybind table, and setup:
[docs/human.md](docs/human.md).

## Features

- [x] Headless / background / focused as a *per-window* property, switchable live
- [x] Human hand-off: `hwatu focus <id>` drops the live session into your tiling WM
- [x] Pixel-diff scoring: match percent + diff regions + heatmap (`diff`)
- [x] Animations as numbers: duration, easing, velocity (`motion`)
- [x] Deterministic animation frames: pin all animations at time t (`seek`)
- [x] Page state as JSON, tokens not pixels (`snapshot`)
- [x] Real input events with structured errors (`click` / `type` / `scroll` / `upload`)
- [x] JS errors, console output, failed requests (`console`)
- [x] Push event subscriptions as JSON lines or MCP notifications (`watch`)
- [x] One-call page assertions with polling (`expect`)
- [x] CAPTCHA / anti-bot detection with structured wait/resume (`challenge`)
- [x] MCP server, plain CLI, and a 1-line JSON socket protocol
- [x] A real browser for humans: mainstream keybinds, media-correct video, native ad blocking, crash restore

## Why not Playwright or chrome-devtools-mcp?

There are three ways to give an agent a browser, and two of them are bad at it:

| | How it runs | What it costs the agent loop |
|---|---|---|
| **Cold library** (Playwright, launched per task) | engine starts when the script does | fast to *call*, slow to *run*: every check pays engine startup; no state survives between tasks |
| **Warm browser** (your Chrome + devtools-mcp) | a full human browser stays resident | resources spent on tabs, extensions, sync, UI you never render, and its windows steal *your* focus while you work |
| **hwatu** | **"the coldest warm daemon"**: engine hot, everything else absent | 8 ms spawns, 35 ms verified checks, invisible until *you* ask to see it (`focus`), interruptible in both directions |

hwatu keeps exactly what makes checks instant (engine, GPU context,
compiled adblock, a prewarmed WebView) and nothing that serves a
human sitting in front of it *unless that human asked for a window*.
That's why it idles warm without a tab bar, and why a kept-warm
Playwright server driven the same way still costs 341 ms per client
to hwatu's 39 ([benchmarks](docs/benchmarks.md)).

The second difference is what comes back. Playwright and
chrome-devtools-mcp are, at their core, automation APIs: they let an
agent *drive* a browser, then hand back raw screenshots and DOM for
the agent to eyeball. hwatu is a *verification* browser: the
measurement primitives are built in, and the browser itself is a warm
daemon where a window costs 13 ms and headless is a window property,
not a launch mode.

The same pass through Playwright's warm in-process CDP connection,
its best case, is 82 ms and five API calls. Shaped like hwatu
actually runs (a fresh client each check against a kept-warm engine),
Playwright's pass is **341 ms vs hwatu's 39**: hwatu is a warm daemon
by design, Playwright is a library you have to keep warm yourself.

## How hwatu compares

**Legend:** ✅ Yes / built-in  ·  🟡 Partial / limited  ·  ❌ No

| Capability | Playwright | chrome-devtools-mcp | hwatu |
| --- | :---: | :---: | :---: |
| Verify pass (load + eval + screenshot), warm in-process | 82 ms | n/a | **35 ms** |
| Verify pass as a warm *service* (fresh client per check) | 341 ms | n/a | **39 ms** |
| Tool calls per verify pass | 5 | 5 | **1** |
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
> corrections are welcome. Honest caveats: Playwright still wins
> cold start (190 vs 435 ms, paid once per boot) and memory; hwatu
> renders WebKit not Chromium (keep a Playwright matrix in CI for
> engine-specific bugs), and it is Linux-only today. Full
> head-to-head data and methodology:
> [docs/benchmarks.md](docs/benchmarks.md).

## Feedback

Tried hwatu? A successful check, a failed install, a missing keybind,
and a site that broke are all useful signals. Share a two-minute
[use report](https://github.com/hongnoul/hwatu/issues/new?template=use-report.yml)
or [report a bug](https://github.com/hongnoul/hwatu/issues/new?template=bug-report.yml).

---

AGPL-3.0 licensed. Linux. WebKitGTK 6.
