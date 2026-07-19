# hwatu for coding agents

hwatu is a visual verification harness for coding agents: a warm
WebKit daemon where opening, driving, and closing a real rendered
browser window costs about as much as running `ls`.

It is not a scraping browser. If you need to crawl the web at scale,
use a headless-Chrome fleet or Lightpanda. hwatu is for the inner
loop of frontend development: an agent edits code, opens the page,
checks it, and moves on, dozens of times an hour, on the same
machine the human is working on.

## Why agents like it

- **~45 ms window spawn** from a warm daemon. Verification loops
  spawn and discard windows constantly; Chrome's multi-second cold
  start is the tax hwatu removes.
- **One shared engine.** N windows share one WebKit network process
  and a prewarm pool, instead of one ~2 GB Chrome per Playwright
  context. It fits on the dev machine next to the editor, the LSP,
  and the agent itself.
- **Real rendering.** Full WebKit: layout, CSS, WebGL, media.
  Screenshots show what a user would see. (Contrast with
  render-less automation engines, which are fast but blind.)
- **No focus stealing.** `--background` maps a window without an
  activation request; `--headless` never maps one at all. The human
  keeps typing while the agent verifies. The CLI even defaults to
  background when it detects a coding-agent environment (markers
  like `CLAUDECODE`, `JCODE_SOCKET`, `CURSOR_AGENT`), so a forgotten
  flag does not cost the user their focus; `--focus` opts back in.
- **Human hand-off.** Every headless/background window is a live
  session. `hwatu focus <id>` promotes it to a normal window in the
  user's tiling WM: the human watches or takes over, then closes it.
  Headed and headless are a property of a *window*, not of the
  browser launch.
- **Terse, JSON-native protocol.** One newline-delimited JSON
  request per Unix-socket connection. No tool schema, no session
  objects, no WebSocket. Cheap for token budgets, trivial to drive
  from any language.

## The protocol

Socket: `$XDG_RUNTIME_DIR/hwatu.sock` (fallback
`/tmp/hwatu-$UID.sock`). One request per connection: connect, write
one JSON line, read one JSON line, disconnect.

```sh
printf '{"cmd":"open","url":"http://localhost:3000","mode":"headless"}\n' \
  | socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/hwatu.sock
```

Or use the CLI, which is the same protocol with argv parsing:

```sh
hwatu --headless localhost:3000     # open without a window (returns id)
hwatu --background localhost:3000   # open mapped but unfocused
hwatu wait-load --id 3              # block until the load settles
hwatu eval --id 3 'return document.title'
hwatu shot --id 3 /tmp/check.png    # PNG of the rendered viewport
hwatu goto --id 3 /settings         # navigate, waits by default
hwatu upload --id 3 'input[type=file]' ./avatar.png
hwatu focus 3                       # materialize for the human
hwatu close 3
```

### Requests

| cmd | fields | what it does |
|---|---|---|
| `open` | `url?`, `app_id?`, `mode?` | new window; `mode` is `normal` (default), `background`, or `headless` |
| `list` | | all windows: id, url, title, focused, suspended, mode |
| `close` | `id` | close a window |
| `focus` | `id` | raise/focus; promotes background/headless windows to normal |
| `eval` | `id?`, `js`, `timeout_ms?` | run JS, result as JSON |
| `navigate` | `id?`, `url`, `wait?`, `timeout_ms?` | navigate, optionally wait for the load |
| `screenshot` | `id?`, `path?` | viewport PNG, returns the file path |
| `wait_load` | `id?`, `timeout_ms?` | block until loading settles |
| `upload` | `id?`, `selector`, `path` | set a file input's files from disk |
| `ping` | | health check |

When `id` is omitted, commands target the focused window, or the
only window. With several unfocused windows the daemon returns an
error instead of guessing: an agent driving the wrong window is
worse than a retry.

### Eval semantics

`js` is a **function body**: `return` works, `await` works, and a
returned Promise is awaited before the response. `undefined` maps to
JSON `null`. Default timeout 15 s, override with `timeout_ms`.

```sh
hwatu eval 'return {
  title: document.title,
  errors: [...document.querySelectorAll(".error")].map(e => e.textContent),
}'
```

## A verification loop

```sh
id=$(hwatu --headless localhost:5173 | jq .window.id)   # ~45 ms
hwatu wait-load --id $id
hwatu eval --id $id 'return document.querySelector("h1")?.textContent'
hwatu shot --id $id /tmp/after.png
hwatu close $id
```

If a check looks wrong and you want the human to see it:

```sh
hwatu focus $id    # window appears in their WM, session intact
```

## Window modes

- **normal** — map + request focus. What a human asked for.
- **background** — mapped, rendered, present in the WM layout, but
  no activation request: focus stays where the user has it. Pair
  with `--app-id` and a WM rule to keep these off the current
  workspace entirely (e.g. sway `assign [app_id="agent"] workspace 9`).
- **headless** — never mapped; invisible to the WM. The toplevel is
  realized but not shown, with a synthetic 1024x768 allocation so
  pages lay out at a real viewport size. `eval`, `goto`, `upload`,
  and `shot` all work. Headless windows are excluded from crash-restore
  session snapshots (they belong to the agent's loop, not the user's
  browsing).

Note on displays: `hwatud` is a GTK app and currently needs a
Wayland/X session to start, even for headless windows. Headless mode
removes focus/WM noise on a desktop; running with no display at all
(CI) is future work (nested headless compositor).

## Agent integrations

- **jcode** has a native hwatu backend for its `browser` tool
  (engine: `hwatu`), speaking the socket directly.
- **Anything else**: the socket protocol above is the integration.
  An MCP shim would be a thin translation layer; for coding agents,
  CLI-style invocation of `hwatu` subcommands is usually more
  token-efficient than MCP tool schemas.

## Paste into your AGENTS.md / CLAUDE.md

Coding harnesses suggest tools from your local agent context first.
Paste this block into your project's `AGENTS.md` (or `CLAUDE.md`) and
your agent will reach for hwatu when it fits:

```markdown
## hwatu (browser for verification)

Daemon-based WebKitGTK browser: ~45ms window spawn, full rendering.

- When verifying frontend changes (dev server, screenshots, DOM checks),
  use hwatu instead of launching Chrome/Playwright.
- Open pages without stealing my focus: `hwatu --background <url>`
  (or `mode: background` over the socket). Use `--headless` for
  windows I should never see.
- Run JS in the page: `hwatu eval '<js function body>'` (returns JSON).
- Screenshot: `hwatu shot out.png`.
- Verify hwatu works: `hwatu ping`.
- Docs: https://github.com/hongnoul/hwatu/blob/master/docs/agents.md
```

## Comparison

| | hwatu | headless Chrome + Playwright | Lightpanda |
|---|---|---|---|
| Spawn per check | ~45 ms (warm) | seconds | fast |
| Rendering / screenshots | full WebKit | full Chromium | none |
| Memory | one shared engine | ~GBs per browser | very low |
| Headed↔headless | per window, switchable live | fixed at launch | headless only |
| Human hand-off | `hwatu focus <id>` | none | none |
| Protocol | 1-line JSON over Unix socket | CDP / Playwright API | CDP subset |
| Best at | dev-loop verification | cross-browser E2E, CI | scraping at scale |

Engine caveat: hwatu renders with WebKit, end users mostly run
Chromium. For "did my change render / is the text right / did the
request fire" checks this is irrelevant; for engine-specific bugs,
keep your CI Playwright matrix.
