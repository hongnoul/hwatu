<div align="center">

# hwatu

[![Latest Release](https://badgen.net/github/release/hongnoul/hwatu?icon=github)](https://github.com/hongnoul/hwatu/releases)
[![License: AGPL-3.0](https://img.shields.io/badge/license-AGPL--3.0-blue?style=flat-square)](LICENSE)
[![CI](https://github.com/hongnoul/hwatu/actions/workflows/ci.yml/badge.svg)](https://github.com/hongnoul/hwatu/actions/workflows/ci.yml)

**Make your agent harness loop instantly faster by giving it real eyes**

</div>

- **STOP your agent claiming "pixel-perfect." Make it prove 97.49%.**
- **STOP paying 5 tool calls per page check. `hwatu check` is one call, ~35 ms (beats warm-server Playwright ~9x).**
- **STOP browser windows stealing your focus. Headless by default, you keep typing.**
- **STOP shipping 170 MB of Chromium. One static binary + your distro's webkitgtk.**

<a href="https://github.com/hongnoul/hwatu/releases/download/readme-assets/demo-shortform.mp4"><img src="https://github.com/hongnoul/hwatu/releases/download/readme-assets/demo-shortform.webp" alt="hwatu daily driving: quarter-width window spawns, buttery Chromium-curve scrolling, and one-keypress-one-reel shortform controls on Instagram Reels" width="800"></a>

## Documents

- [Vision](VISION.md): durable product principles, native platform strategy, swarm model
- [Agent guide](docs/agents.md): protocol, primitives, verification loops
- [Human guide](docs/human.md): a primary browser for tiling WMs, and the hand-off side
- [Benchmarks](docs/benchmarks.md): every number, measured, with methodology
- [Roadmap](docs/roadmap.md): plan of record, priorities, non-goals
- [Continuous improvement](docs/continuous-improvement.md): activation metric, feedback loop, weekly cadence
- [Launch kit](docs/launch-kit.md): reusable copy, channels, and measurement plan

## Quick Start

**Install → Detect workflow → Connect agent → Verify page → Hand off to human**

```bash
curl -fsSL https://raw.githubusercontent.com/hongnoul/hwatu/main/scripts/install.sh | bash
hwatu setup
```

One static binary plus your distro's `webkitgtk-6.0` (the installer
checks). On Arch: `yay -S hwatu`. From source: `cargo build --release`.

The installer installs binaries only. `hwatu setup` detects supported coding
agents and prints the available connections without changing their config.
Choose a client explicitly when you are ready:

```sh
hwatu doctor
hwatu setup --client claude --scope project --dry-run
hwatu setup --client claude --scope project
hwatu demo
```

- **Install:** download two binaries and check WebKitGTK.
- **Detect:** find Claude Code, Cursor, Jcode, or a generic MCP workflow.
- **Connect:** use Jcode's native socket, MCP, or the CLI fallback.
- **Verify:** run a headless rendered smoke test with `doctor` or `demo`.
- **Hand off:** materialize the same live session only when a human is needed.

Setup is previewable, idempotent, and reversible with the same client and
scope plus `--undo`. Project scope creates shareable configuration; user scope
keeps it personal. Claude Code asks each user to approve project-scoped MCP
servers when it next starts.

Manual MCP configuration remains one portable entry:

```json
{ "mcpServers": { "hwatu": { "command": "hwatu", "args": ["mcp"] } } }
```

Or skip MCP entirely: every command is a short CLI call or one
newline-delimited JSON line over a Unix socket.

```sh
hwatu localhost:3000       # open a window like you open a terminal
```

Tried hwatu? A successful check, a failed install, and a missing workflow are
all useful signals. Share a two-minute [use report](https://github.com/hongnoul/hwatu/issues/new?template=use-report.yml)
or [report a bug](https://github.com/hongnoul/hwatu/issues/new?template=bug-report.yml).

## Tell your agent to use Hwatu

Connecting Hwatu makes its tools available. A project instruction tells the
agent when to use them and what evidence counts as success. Add this to
`AGENTS.md`, `CLAUDE.md`, Cursor rules, or the equivalent for your harness:

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

The same instruction works with Hwatu's MCP tools, short CLI commands, or
Jcode's native integration. Name the user journey and the observable result
that proves it worked. See the full [agent guide](docs/agents.md), including a
larger copy-paste policy and verification loops.

## Features

- [x] Pixel-diff scoring: match percent + diff regions + heatmap (`diff`)
- [x] Animations as numbers: duration, easing, velocity (`motion`)
- [x] Deterministic animation frames: pin all animations at time t (`seek`)
- [x] Page state as JSON, tokens not pixels (`snapshot`)
- [x] Real input events with structured errors (`click` / `type` / `scroll` / `upload`)
- [x] JS errors, console output, failed requests (`console`)
- [x] Push event subscriptions as JSON lines or MCP notifications (`watch`)
- [x] One-call page assertions with polling (`expect`)
- [x] Headless / background / focused as a *per-window* property, switchable live
- [x] Human hand-off: `hwatu focus <id>` drops the live session into your tiling WM
- [x] CAPTCHA / anti-bot detection with structured wait/resume (`challenge`)
- [x] MCP server, plain CLI, and a 1-line JSON socket protocol
- [x] A real browser for humans: mainstream keybinds, media-correct video, native ad blocking, crash restore

## A browser for your tiling WM

hwatu is agent-first, but as of v0.7.0 the human side stopped being a
viewer and became a browser you can actually live in on Hyprland,
sway, or niri:

- **Your WM is the tab bar.** No tabs, no chrome: `hwatu <url>` opens
  a window like your terminal opens a shell, at a third of the
  monitor width, and your tiler does the rest. Ready-made configs:
  [hyprland](examples/hyprland.conf), [sway](examples/sway.config),
  [niri](examples/niri.kdl).
- **Mainstream keybinds, all rebindable.** `ctrl+l` edits the URL,
  `ctrl+f` finds, `ctrl+k` opens a fuzzy command palette,
  `~/.config/hwatu/keys.conf` overrides anything.
- **Video actually works.** Unmuted autoplay, playback that survives
  focus loss, and a blur-shield that took YouTube Shorts from ~34 to
  ~95 fps. One shortform control scheme (arrows snap exactly one
  video, Space pauses, hold ArrowRight for 2x) across Reels, Shorts,
  and TikTok.
- **Native ad blocking.** EasyList compiled into WebKit's
  content-extension engine: ~119k rules, zero JS in the request path,
  no extension process.
- **Chromium-curve scrolling** for wheel and keys, replacing
  WebKitGTK's isolated-pulse scroller.
- **One warm engine.** Every window shares the daemon (~56 MB per
  extra window), suspends when unfocused, and crash-restores at its
  last URL.

Details, keybind table, and setup: [docs/human.md](docs/human.md).

## Why not Playwright or chrome-devtools-mcp?

There are three ways to give an agent a browser, and two of them are bad at it:

| | How it runs | What it costs the agent loop |
|---|---|---|
| **Cold library** (Playwright, launched per task) | engine starts when the script does | fast to *call*, slow to *run*: every check pays engine startup; no state survives between tasks |
| **Warm browser** (your Chrome + devtools-mcp) | a full human browser stays resident | resources spent on tabs, extensions, sync, UI you never render, and its windows steal *your* focus while you work |
| **hwatu** | **"the coldest warm daemon"**: engine hot, everything else absent | 8 ms spawns, 35 ms verified checks, invisible until *you* ask to see it (`focus`), interruptible in both directions |

hwatu keeps exactly what makes checks instant (engine, GPU context,
compiled adblock, a prewarmed WebView) and nothing that serves a
human sitting in front of it. That's why it idles warm without a tab
bar, and why a kept-warm Playwright server driven the same way still
costs 341 ms per client to hwatu's 39 ([benchmarks](docs/benchmarks.md)).

The second difference is what comes back. Playwright and
chrome-devtools-mcp are, at their core, automation
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
[scripts/demo/](scripts/demo/). The README hero uses a second, real-agent
scenario against AIUC: four responsive viewport diffs followed by live human
hand-off. Reproduce that take, including its evidence manifests, from
[scripts/demo-aiuc/](scripts/demo-aiuc/). A full verification pass (open, load,
eval, screenshot, close) is **one command, one tool call, ~35 ms
median** ([benchmarks](docs/benchmarks.md)):

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
`notifications/hwatu/event`. Each connection gets a strictly monotonic
sequence starting at zero; closing or stalling the connection drops its
subscription without blocking the daemon.

The same pass through Playwright's warm in-process CDP connection,
its best case, is 82 ms and five API calls. Shaped like hwatu
actually runs (a fresh client each check against a kept-warm
engine), Playwright's pass is **341 ms vs hwatu's 39**: hwatu is a
warm daemon by design, Playwright is a library you have to keep warm
yourself.

And when the agent hits a CAPTCHA or a judgment call, `hwatu focus`
materializes its live session, cookies and state intact, in your
tiling WM. You act for ten seconds. It takes back over. **This is
the adjective no other tool gets to claim: interruptible.**
Everywhere else, headless is decided at launch and a human can never
see the session at any price. In hwatu it's a window property,
switchable live, in both directions.

The speed follows from the same design decision: hwatu is a **warm
daemon**, not a library you launch. The engine, the GPU context, the
compiled adblock ruleset, and a prewarmed WebView outlive every
task, so a check starts from a hot pipeline instead of a cold
process. Playwright is a library and cold by nature. Keeping it
warm is something *you* build (a server process, connection
management, context pooling); hwatu ships warm as the default and
the only mode.

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

---

AGPL-3.0 licensed. Linux. WebKitGTK 6.
