# Hwatu vision

Status: the constraint set. It changes rarely and only on purpose. The plan
of record is [`docs/roadmap.md`](docs/roadmap.md). The cross-platform
execution plan is
[`docs/roadmaps/native-platforms.md`](docs/roadmaps/native-platforms.md).

## One sentence

Hwatu gives every agent its own invisible, real browser on the user's
machine, driven by portable skills, on Linux, macOS, and Windows alike.

## Problem

Agents need the web, and today's options make them pay for it.

- **Automation frameworks** (Playwright, CDP) are built for test suites. One
  page check costs five calls and hundreds of milliseconds.
- **Extensions in the human's browser** share the human's focus, tabs, and
  logins. We measured a production bridge stealing focus on three of six
  core actions.
- **Cloud browser services** move the session off the user's machine, meter
  every page, and sell spoofed fingerprints as identity.

## The model: one browser, one skill format

Hwatu has two parts.

1. **The browser.** A warm, headless, native engine per OS behind one
   versioned protocol.
2. **Skills.** Portable recipes the agent runs against that browser.

Checking a UI and ordering dinner are the same activity: open pages, act on
them, observe the result. Hwatu does not split them into two products or two
syntaxes. Every job is a skill, and every skill is built from one step
vocabulary.

| Step kind | Examples | Used for |
| --- | --- | --- |
| act | `navigate`, `click`, `type`, `fill`, `scroll`, `upload` | driving the page |
| observe | `snapshot`, `content`, `eval`, `shot`, `console`, `net` | reading the page as data |
| assert | `expect`, `diff`, `motion` | proving an outcome with a measurement |
| human | `ask`, `confirm`, `show` | the only places a person is involved |

A UI check is a skill that ends in `assert` steps and returns evidence. A
DoorDash order is a skill that ends in a `confirm` step and stops for a
human yes. Same format, same runner, same report.

A skill is a single file: a short description of intent for the agent, plus
step blocks in the shared vocabulary. The runner executes steps in one call,
waits on the page instead of sleeping, and returns a per-step report. When a
step fails, the agent reads the report, adapts with the same vocabulary, and
resumes. Prose carries judgment. Steps carry everything mechanical.

Today's `hwatu verify` specs and `hwatu batch` arrays are early forms of
this format and converge into it.

### Human steps

The human appears only through three explicit steps.

- **`ask`** requests what only a person can give: a credential, a one-time
  code, a choice. Credentials pass through to the page once. The session
  persists, so the agent asks once per site, not once per run.
- **`confirm`** gates anything irreversible: paying, sending, deleting. It
  needs a fresh yes in the current session. A page can never satisfy it.
- **`show`** renders the live session on request, as a screenshot, a diff,
  or a visible view of the real page, then returns to headless with nothing
  lost. Seeing is pull, never push.

A challenge that cannot become an `ask`, such as a CAPTCHA, ends the run with
a structured block. The session stays intact for the human.

## Outcome

On any of the three operating systems, a user can:

1. install Hwatu the normal way for their OS,
2. run `hwatu doctor` and see green,
3. drop a skill into their agent's skills folder,

and the skill runs. The same file works unmodified everywhere. If a skill
needs an `if macOS` branch, Hwatu has a bug.

## Constitution

- **Headless is the invariant.** Every step works without a visible window.
  Visibility exists only through `show`.
- **Skills are the interface.** The CLI, MCP, and every integration are
  adapters over one versioned protocol in `hwatu-ipc`, and skills target that
  protocol, never an OS or an engine.
- **Measurements, not impressions.** Observations and assertions return
  data: scores, diffs, timings, structured errors. "Looks right" is not a
  result.
- **Credentials pass through.** Hwatu keeps no password store of its own. It
  may read from a manager the user already runs. It never logs or retains a
  secret.
- **Irreversible means `confirm`.** No skill, page, or default can skip it.
- **Warm by default.** Engine and views stay warm between runs.
- **Local-first.** Everything runs on the user's machine. No relay sees the
  session. No metering shapes behavior.
- **Native engines.** WebKitGTK on Linux, WKWebView on macOS, WebView2 on
  Windows. No bundled Chromium.
- **Honest parity.** Engines differ. Hwatu guarantees step semantics and
  reports per-engine capabilities. A step an engine cannot perform fails with
  a stable error. It never silently succeeds or changes meaning by platform.

## Architecture boundary

Everything a skill can observe is portable. Native code exists only to host
an invisible view, deliver trusted input to it, and capture its pixels.

| Portable, written once | Native, one per OS |
| --- | --- |
| protocol, capability model, skill runner | UI loop and process lifecycle |
| CLI and MCP adapters | transport: Unix socket or named pipe |
| page scripts: snapshot ranking, element matching, content extraction | view hosting, trusted input |
| diffing, assertions, reports | pixel capture, engine events |
| profiles and granted sessions | storage location |

Portable code never imports GTK, GLib, Cocoa, Win32, or engine types, and it
compiles and tests on all three CI runners without native SDKs.

## Support is earned, not compiled

A platform is supported only when a clean OS image can install Hwatu, pass
`hwatu doctor`, pass the shared conformance fixtures, and run an unmodified
reference skill end to end. That skill is an authenticated, multi-step task
stopped at `confirm`. Until then, README and release metadata do not claim
the platform.

## Not doing

- A browser for humans: no tabs, bar, keybinds, themes, or reader mode.
- A bundled Electron, CEF, or private Chromium.
- An extension inside the user's daily browser.
- Stored credentials.
- Fingerprint spoofing, CAPTCHA solving, or bypassing access controls.
- A cloud relay between agent and page.
- Per-OS skills, CLIs, or MCP tools.
- A second syntax for verification versus automation.
- Pixel-identical rendering across engines.

## Decision test

A change belongs in Hwatu if it makes skills run more reliably, on more
machines, with fewer human steps, without weakening the constitution. If it
needs a visible window, keeps a secret, makes a skill OS-aware, or adds a
second way to express the same job, it is not progress.
