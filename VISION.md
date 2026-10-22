# Vision

Status: constraint set. Changes are rare and deliberate. Plan of record:
[`docs/roadmap.md`](docs/roadmap.md). Cross-platform execution plan:
[`docs/roadmaps/native-platforms.md`](docs/roadmaps/native-platforms.md).

## Summary

A headless browser that runs on the user's machine and is driven by agents
through a CLI and MCP. Target platforms: Linux, macOS, Windows.

## Problem

- Automation frameworks (Playwright, CDP) are designed for test suites. A
  single page check takes about five calls and several hundred milliseconds.
- Browser extensions run inside the user's browser and share its focus,
  tabs, and logins. A measured production bridge took focus on three of six
  core actions.
- Cloud browser services run the session off the user's machine, bill per
  page, and use spoofed fingerprints.

## Scope

The project provides the browser and its command set. It does not provide
skills.

Skills are the plain-text skill files that agent harnesses already support,
such as a `SKILL.md` in the harness's skills folder. The harness stores,
discovers, and loads them. A skill that uses this browser contains `hwatu`
commands. There is no skill library, registry, file format, or storage in
this project.

UI verification and task automation use the same commands:

| Kind | Commands | Purpose |
| --- | --- | --- |
| act | `goto`, `click`, `type`, `fill`, `scroll`, `upload` | change page state |
| observe | `snapshot`, `content`, `eval`, `shot`, `console`, `net` | read page state as data |
| assert | `expect`, `diff`, `motion` | check page state against an expected value |
| human | `show`, structured stops | points where a person is involved |

A UI-check skill ends with `assert` commands and returns their results. A
DoorDash skill ends at checkout and waits for user confirmation. A sequence
of commands can run in one `hwatu batch` call, which waits on page state
instead of fixed delays and returns a result per step.

### Human involvement

The browser does not communicate with the user. It stops and returns a
structured reason. The agent relays the reason through its harness.

- Login or one-time code: the browser reports the login wall. The agent asks
  the user and enters the value once. The session persists across runs.
- Irreversible action (payment, send, delete): requires a new confirmation
  from the user in the current session. Page content cannot provide it.
- `show`: on request, renders the live session as a screenshot, a diff, or a
  visible window. The session returns to headless afterward without state
  loss. The browser never becomes visible unless requested.

CAPTCHAs and anti-bot challenges are handled with the standard commands.
`challenge` detects them, `shot` and `snapshot` read them, and `click
--trusted` and `type` answer them in the real engine. If the agent cannot
clear a challenge, `challenge` returns a structured block, the session is
kept, and the agent can hand off to the user through `show`.

## Outcome

On Linux, macOS, and Windows, a user can:

1. install the package for their OS,
2. run `hwatu doctor` with no failures,
3. add a skill to their agent's skills folder,

and the skill runs. The same skill file works on all three OSes without
changes. A skill that requires OS-specific branches indicates a defect.

## Constitution

- Headless only. The browser is visible only through `show`.
- Commands are the interface. The CLI, MCP, and integrations are adapters
  over one versioned protocol in `hwatu-ipc`. Skills call commands and do
  not depend on the OS or engine.
- Results are data: scores, diffs, timings, structured errors.
- No password storage. Credentials are entered once and not retained.
- Local only. No relay or metering.
- Native engines: WebKitGTK on Linux, WKWebView on macOS, WebView2 on
  Windows. No bundled Chromium.

## Architecture boundary

Behavior visible through commands is portable. Native code hosts an
invisible view, delivers trusted input to it, and captures its pixels.

| Portable, written once | Native, one per OS |
| --- | --- |
| protocol, capability model, batch execution | UI loop and process lifecycle |
| CLI and MCP adapters | transport: Unix socket or named pipe |
| page scripts: snapshot ranking, element matching, content extraction | view hosting, trusted input |
| diffing, assertions, reports | pixel capture, engine events |
| profiles and granted sessions | storage location |

Portable code does not import GTK, GLib, Cocoa, Win32, or engine types. It
compiles and passes tests on all three CI runners without native SDKs.

## Platform support criteria

A platform is supported when, on a clean OS image:

1. the package installs,
2. `hwatu doctor` passes,
3. the shared conformance fixtures pass,
4. an unmodified reference skill runs end to end from a stock agent harness.
   The reference skill is an authenticated, multi-step task that stops
   before an irreversible step.

README and release metadata list a platform only after it meets these
criteria. Cross-compilation alone does not count.

## Not doing

- A browser for humans: tabs, address bar, keybinds, themes, reader mode.
- A skill library, registry, file format, or store.
- Bundled Electron, CEF, or Chromium.
- An extension in the user's browser.
- Stored credentials.
- Fingerprint spoofing, third-party CAPTCHA solver services, injected
  challenge tokens, or bypassing access controls.
- A cloud relay between agent and page.
- Per-OS commands, CLIs, or MCP tools.
- Separate command sets for verification and automation.
- Pixel-identical rendering across engines.

## Decision test

A change is in scope if it makes skills that call `hwatu` more reliable,
supported on more machines, or less dependent on user input, and it does not
violate the constitution. A change is out of scope if it requires a visible
window, stores a secret, requires OS-specific skills, adds a second way to
express the same task, or adds skill storage.
