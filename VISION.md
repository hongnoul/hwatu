# Hwatu vision

Status: the constraint set. It changes rarely and only on purpose. The plan
of record is [`docs/roadmap.md`](docs/roadmap.md). The cross-platform
execution plan is
[`docs/roadmaps/native-platforms.md`](docs/roadmaps/native-platforms.md).

## One sentence

Hwatu gives every agent its own invisible, real browser on the user's
machine, driven by ordinary agent skills, on Linux, macOS, and Windows alike.

## Problem

Agents need the web, and today's options make them pay for it.

- **Automation frameworks** (Playwright, CDP) are built for test suites. One
  page check costs five calls and hundreds of milliseconds.
- **Extensions in the human's browser** share the human's focus, tabs, and
  logins. We measured a production bridge stealing focus on three of six
  core actions.
- **Cloud browser services** move the session off the user's machine, meter
  every page, and sell spoofed fingerprints as identity.

## The model: Hwatu is the browser, skills stay in the harness

Hwatu ships one thing: a headless native browser per OS, driven by one set of
commands.

Skills are not Hwatu's. They are the plain-text skill files every agent
harness already supports (for example a `SKILL.md` in the harness's skills
folder), stored, discovered, and loaded by the harness the way it loads any
other skill. Hwatu has no skill library, registry, format, or storage. A
skill that uses Hwatu is just a skill whose steps call `hwatu` commands.

Checking a UI and ordering dinner are the same activity: open pages, act on
them, observe the result. So there is one command vocabulary for both, not a
verification API and a separate automation API.

| Kind | Commands | Used for |
| --- | --- | --- |
| act | `goto`, `click`, `type`, `fill`, `scroll`, `upload` | driving the page |
| observe | `snapshot`, `content`, `eval`, `shot`, `console`, `net` | reading the page as data |
| assert | `expect`, `diff`, `motion` | proving an outcome with a measurement |
| human | `show`, plus structured stops | the only places a person is involved |

A UI-check skill ends in `assert` commands and reports evidence. A DoorDash
skill ends at checkout and stops for a human yes. Both are ordinary skill
files, written in prose with `hwatu` commands inline. Prose carries judgment.
Commands carry everything mechanical. A run of mechanical commands can go in
one `hwatu batch` call that waits on the page instead of sleeping and returns
a per-step report.

### Where the human comes in

Hwatu never talks to the human. It stops with a structured reason, and the
agent relays it through its own harness.

- **Login or one-time code.** Hwatu reports the wall. The agent asks the
  human, then passes the value through to the page once. The session
  persists, so the human is asked once per site, not once per run.
- **Irreversible action.** Paying, sending, and deleting need a fresh yes in
  the current session. A page can never supply it.
- **`show`.** On request, the live session renders as a screenshot, a diff,
  or a visible view of the real page, then returns to headless with nothing
  lost. Seeing is pull, never push.

A challenge the human cannot answer as text, such as a CAPTCHA, ends the run
with a structured block. The session stays intact for the human.

## Outcome

On any of the three operating systems, a user can:

1. install Hwatu the normal way for their OS,
2. run `hwatu doctor` and see green,
3. drop a skill into their agent's skills folder,

and the skill runs. The same file works unmodified everywhere. If a skill
needs an `if macOS` branch, Hwatu has a bug.

## Constitution

- **Headless is the invariant.** Visibility exists only through `show`.
- **Commands are the interface.** The CLI, MCP, and every integration are
  adapters over one versioned protocol in `hwatu-ipc`. Skills call those
  commands, never an OS or an engine.
- **Measurements, not impressions.** Observations and assertions return
  data: scores, diffs, timings, structured errors.
- **Credentials pass through.** Hwatu keeps no password store of its own.
- **Local-first.** Everything runs on the user's machine. No relay sees the
  session. No metering shapes behavior.
- **Native engines.** WebKitGTK on Linux, WKWebView on macOS, WebView2 on
  Windows. No bundled Chromium.

## Architecture boundary

Everything a command can observe is portable. Native code exists only to host
an invisible view, deliver trusted input to it, and capture its pixels.

| Portable, written once | Native, one per OS |
| --- | --- |
| protocol, capability model, batch execution | UI loop and process lifecycle |
| CLI and MCP adapters | transport: Unix socket or named pipe |
| page scripts: snapshot ranking, element matching, content extraction | view hosting, trusted input |
| diffing, assertions, reports | pixel capture, engine events |
| profiles and granted sessions | storage location |

Portable code never imports GTK, GLib, Cocoa, Win32, or engine types, and it
compiles and tests on all three CI runners without native SDKs.

## Support is earned, not compiled

A platform is supported only when a clean OS image can install Hwatu, pass
`hwatu doctor`, pass the shared conformance fixtures, and run an unmodified
reference skill end to end from a stock agent harness. That skill is an
authenticated, multi-step task that stops before the irreversible step.
Until then, README and release metadata do not claim the platform.

## Not doing

- A browser for humans: no tabs, bar, keybinds, themes, or reader mode.
- A skill library, registry, file format, or store. Skills live wherever the
  user's agent harness keeps them.
- A bundled Electron, CEF, or private Chromium.
- An extension inside the user's daily browser.
- Stored credentials.
- Fingerprint spoofing, CAPTCHA solving, or bypassing access controls.
- A cloud relay between agent and page.
- Per-OS commands, CLIs, or MCP tools.
- Separate vocabularies for verification and automation.
- Pixel-identical rendering across engines.

## Decision test

A change belongs in Hwatu if it makes skills that call `hwatu` run more
reliably, on more machines, with fewer human stops, without weakening the
constitution. If it needs a visible window, keeps a secret, makes a skill
OS-aware, adds a second way to express the same job, or pulls skill storage
into Hwatu, it is not progress.
