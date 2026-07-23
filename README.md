# hwatu

[![Latest Release](https://badgen.net/github/release/hongnoul/hwatu?icon=github)](https://github.com/hongnoul/hwatu/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](LICENSE)
[![CI](https://github.com/hongnoul/hwatu/actions/workflows/ci.yml/badge.svg)](https://github.com/hongnoul/hwatu/actions/workflows/ci.yml)

**The verification browser for coding agents.** hwatu turns "does the
page look right" into a number an agent can climb, and turns invisible
agent sessions into real windows on your screen the moment a human is
needed.

Your agent works in windows that don't exist on your desktop: no focus
stolen, no WM pollution, at any parallelism. It measures what it
builds: pixel-diff scores, animation curves as numbers, screenshots of
the *middle* of an animation. And when it hits a CAPTCHA or a judgment
call, `hwatu focus` materializes its live session, cookies, state and
all, in your tiling WM. You act for ten seconds. It takes back over.

![hwatu spawning windows in ~48ms from a warm daemon](docs/assets/spawn-demo.svg)

## The loop

An agent iterating a page toward a design reference, real commands,
real output:

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
agent took it from **85.1% to 98.8% pixel match**, reading easings
from `hwatu motion` and comparing animation frames pinned with
`hwatu seek`. Reproduce it yourself: [scripts/demo/](scripts/demo/).

A full verification pass (open headless, wait for load, eval,
screenshot, close) is **87 ms median**. Windows spawn in **13 ms**
from the warm daemon. Numbers and methodology:
[docs/benchmarks.md](docs/benchmarks.md).

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/hongnoul/hwatu/main/scripts/install.sh | bash
```

One static binary plus your distro's `webkitgtk-6.0` (the installer
checks). No Node, no npm package, no 170 MB browser download. On Arch:
`yay -S hwatu`. From source: `cargo build --release`.

Wire it into any MCP client (Claude Code, Cursor, ...) with one entry:

```json
{ "mcpServers": { "hwatu": { "command": "hwatu", "args": ["mcp"] } } }
```

jcode drives hwatu natively. Everything is also one newline-delimited
JSON request over a Unix socket, so any language and any script can
drive it with zero client library. Full agent guide:
[docs/agents.md](docs/agents.md).

## What the agent gets

| primitive | what it answers |
|---|---|
| `snapshot` | what's on this page, what can I click (JSON, ~tokens not pixels) |
| `diff --other/--baseline` | how close are these two renders, where do they differ, as a score + regions + heatmap |
| `motion` | every animation as numbers: duration, delay, easing, keyframes |
| `seek` | pin all animations at time t; two shots at the same t are byte-identical |
| `expect` | assert page state in one call (polls, structured pass/fail) |
| `shot` / `shot --full` | what a user would see (real GPU-composited WebKit render) |
| `click` / `type` / `scroll` / `upload` | real pointer/input events, structured errors on misses |
| `console` | JS errors, console output, failed requests since last check |
| `challenge` | is this a CAPTCHA / anti-bot wall, should a human take over |
| `resize` | verify responsive layouts across viewport widths |
| `focus <id>` | materialize any headless session as a real window for the human |

Ambiguity is an error with a match count, never a silent wrong click.
Refs from `snapshot` are live element handles; staleness is a clear
error, not a mystery.

## vs the alternatives

| | hwatu | Playwright (headless Chromium) | chrome-devtools-mcp | Percy / Chromatic / Applitools | ditto & site cloners | tterm & browser-in-IDE cockpits |
|---|---|---|---|---|---|---|
| Built for | agent inner loop on your machine | cross-browser E2E test suites | DevTools introspection for agents | CI visual regression gates | one-shot site→code generation | human watching an agent |
| Pixel verification | `diff`: score + regions + heatmap, 87 ms warm pass | `toHaveScreenshot` baselines (test-suite shaped) | screenshots only | mature, but cloud round-trip, priced per shot | none — never renders its own output | none |
| Animations | read as numbers (`motion`), pin mid-flight (`seek`) | disable or fast-forward to end state | raw CDP | disabled to avoid flakes | captured at generation, verified by eyeball | none |
| Focus stealing at N agents | never — headless/background are window properties | headless: fine; headed: every window pops | fine headless | n/a (cloud) | n/a | its own pane |
| Human hand-off mid-session | `focus <id>`: same live session becomes a real WM window | impossible headless; headed costs focus-steal always | none | none | n/a | human is already watching |
| CAPTCHA / needs-human | `challenge` detects + structured wait/resume | manual workarounds | none | n/a | out of scope | human solves in-pane |
| Runtime deps | 1 MB binary + distro webkitgtk | Node + package + ~170 MB browser per version | Node + Chrome | SaaS account | Node + Playwright + service | full app |
| Interface cost for an agent | one JSON line / short CLI / MCP | client library, session objects | MCP over CDP verbosity | API + dashboard | REST/MCP job API | n/a |

Honest caveats, so you don't discover them in a comment section: raw
warm latency vs Playwright is a tie (83 vs 82 ms), and hwatu's
resident memory is *higher* than headless-shell because every hwatu
window is a real GPU-composited, WM-mappable surface, that's the
price of hand-off, partially reclaimed by suspending idle windows
(~56 MB back per window after 120 s). hwatu renders WebKit, not
Chromium: right for "did my change render correctly", wrong for
engine-specific bug hunts, keep a Playwright matrix in CI for those.
Linux-only today. Full head-to-head data:
[docs/benchmarks.md](docs/benchmarks.md).

## The hand-off

The feature the others structurally can't copy: **headless is a window
property, not a launch mode.** Every invisible agent session is a live
window the WM simply hasn't been shown.

```sh
id=$(hwatu --headless --json https://example.com | jq .id)
hwatu challenge --id "$id"
# {"status":"challenge","challenge_type":"turnstile","manual_required":true}
hwatu focus "$id"          # the session appears in your tiler, as it is
hwatu challenge --id "$id" --wait --timeout-ms 60000
# human clicks the checkbox; {"status":"cleared"} — agent resumes
```

Same cookies, same scroll position, same half-filled form. Works for
CAPTCHAs, OAuth consents, 2FA prompts, payment confirmations, and
plain "does this look right to you". `challenge` is detection and
hand-off only, by design: no solver APIs, no token injection, no
fingerprint games.

Agents get headless by default: hwatu detects coding-agent
environments (`CLAUDECODE`, `JCODE_SOCKET`, `CURSOR_AGENT`, ...) so a
forgotten flag never puts a window in your WM. `--focus` opts back
in; `HWATU_AGENT_MODE` / `agent_mode` in
`~/.config/hwatu/config.json` tunes the default.

## For the human on the other side

hwatu is also a deliberately minimal WebKit browser for tiling WMs:
no tabs (your WM tiles are the tabs), no chrome (a vim-style bottom
bar summoned on demand), remappable keybinds (`keys.conf`), built-in
native ad blocking (EasyList compiled into WebKit's content-extension
engine, zero JS in the request path), sane downloads, crash-restore
sessions, TLS and permission prompts as one-line y/n bar prompts.

The human side is intentionally scoped to hand-off quality. If you
want link hints, history completion, and password-manager
integration as a daily driver, qutebrowser is the better choice, and
we say so. Scope and non-goals: [docs/roadmap.md](docs/roadmap.md).

```sh
hwatu                      # launcher (autostarts the daemon)
hwatu example.com          # open a URL
hwatu how to exit vim      # non-URLs become a web search
hwatu list                 # every window: id, url, title
hwatu adblock update       # fetch + compile EasyList/EasyPrivacy
hwatu update               # self-update
hwatu quit                 # stop the daemon
```

## Architecture

```
hwatu <url>  --unix socket-->  hwatud (GTK main loop)
                                 ├── prewarmed WebView (adopted instantly,
                                 │   next one warmed in idle time)
                                 ├── window registry (id -> WebView)
                                 └── WebKit: shared network process,
                                     per-site web processes
```

The daemon owns WebKitGTK 6 and a prewarmed WebView pool; the client
is one Unix-socket roundtrip, the way `emacsclient`/`wezterm` split
the editor. N windows share one engine (~56 MB per extra window);
unfocused windows suspend after 120 s and resume instantly from the
pool. If the daemon dies uncleanly, the next start reopens every
window at its last URL.

Crates: `crates/ipc` (JSON protocol), `crates/hwatud` (daemon, GTK4 +
webkit6), `crates/hwatu` (client, no GTK linkage).

## Docs

- [docs/agents.md](docs/agents.md) — the full agent protocol and guide
- [docs/benchmarks.md](docs/benchmarks.md) — every number, measured, with methodology
- [docs/roadmap.md](docs/roadmap.md) — plan of record, priorities, non-goals
- [scripts/demo/](scripts/demo/) — the stripe convergence demo, reproducible
- [examples/clone/](examples/clone/) — generalized page-clone pipeline

MIT licensed. Linux. WebKitGTK 6.
