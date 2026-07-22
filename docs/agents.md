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

- **13-16 ms window spawn** from a warm daemon, measured medians
  across focused/background/headless modes
  ([benchmarks](benchmarks.md)). Verification loops spawn and
  discard windows constantly; Chrome's multi-second cold start is
  the tax hwatu removes.
- **One shared engine.** N windows share one WebKit network process
  and a prewarm pool: measured, each extra window costs ~56 MB PSS
  on top of the daemon's floor, instead of one multi-hundred-MB
  Chrome per Playwright context. It fits on the dev machine next to
  the editor, the LSP, and the agent itself.
- **Real rendering.** Full WebKit: layout, CSS, WebGL, media.
  Screenshots show what a user would see. (Contrast with
  render-less automation engines, which are fast but blind.)
- **No focus stealing.** `--background` maps a window without an
  activation request; `--headless` never maps one at all. The human
  keeps typing while the agent verifies. The CLI even defaults to
  headless when it detects a coding-agent environment (markers
  like `CLAUDECODE`, `JCODE_SOCKET`, `CURSOR_AGENT`), so a forgotten
  flag never puts a window in the user's WM; `--focus` opts back in,
  and `HWATU_AGENT_MODE` / `"agent_mode"` in
  `~/.config/hwatu/config.json` tune the agent default
  (`normal` | `background` | `headless`).
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
hwatu wait-load                     # block until the load settles
hwatu snapshot                      # text + interactables, cheaper than a shot
hwatu eval 'document.title'         # id-less: follows the window you opened
hwatu click a --contains "Sign in"  # real pointer-event click
hwatu click --ref 4                 # click interactable #4 from the snapshot
hwatu type 'input[name=q]' hi --enter   # fill and submit
hwatu console                       # console.*, exceptions, failed requests
hwatu challenge                     # detect CAPTCHA / anti-bot UI, as JSON
hwatu challenge --wait --timeout-ms 30000  # wait while the user clears it
hwatu shot /tmp/check.png           # PNG of the rendered viewport
hwatu shot --full /tmp/page.png     # PNG of the whole document
hwatu scroll h2 --contains Pricing  # scroll into view, reports what it hit
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
| `screenshot` | `id?`, `path?`, `full?` | PNG of the viewport (`full: true` = whole document), returns the file path |
| `wait_load` | `id?`, `timeout_ms?` | block until loading settles |
| `challenge` | `id?`, `wait?`, `timeout_ms?` | detect CAPTCHA/anti-bot UI; optionally wait for manual/user resolution |
| `scroll` | `id?`, `selector?`, `nth?`, `contains?`, `to_y?`, `by_pages?` | scroll and report where it landed |
| `snapshot` | `id?` | token-cheap page state: url, title, text, indexed interactables |
| `click` | `id?`, `selector?`, `nth?`, `contains?`, `ref?` | click an element (real pointer events), reports what it hit |
| `type` | `id?`, `selector?`/`ref?`, `text`, `clear?`, `enter?` | fill input/textarea/select/contenteditable |
| `console` | `id?`, `clear?`, `limit?` | read the console/error/network capture buffer |
| `upload` | `id?`, `selector`, `path` | set a file input's files from disk |
| `ping` | | health check |

When `id` is omitted, commands target the focused window, else the
window your last automation command targeted (including `open`), else
the only window. So `open` → `eval` → `shot` chains never need an id.
Only genuine ambiguity (several windows, none focused, none driven
yet) is an error: an agent driving the wrong window is worse than a
retry.

### Challenge hand-off

`hwatu challenge` detects common CAPTCHA and anti-bot surfaces and
returns structured JSON for an agent workflow:

```sh
hwatu challenge
{"status":"challenge","challenge_type":"turnstile","confidence":0.5,
 "evidence":[{"kind":"turnstile","detail":"iframe ...","weight":4}],
 "actionable":true,"manual_required":true,"elapsed_ms":0,
 "url":"https://example.com/","title":"Just a moment..."}
```

With `--wait`, hwatu polls until the challenge disappears or the
timeout expires:

```sh
id=$(hwatu --headless --json https://example.com | jq .id)
hwatu challenge --id "$id" --wait --timeout-ms 60000
```

If the page needs a human, the agent can `hwatu focus $id`, tell the
user what to solve, and call `challenge --wait` again or continue once
the returned status is `cleared`. This keeps the same live window and
cookies, so the assigned workflow can resume after the manual step.

This command is detection and hand-off only. It does not solve
CAPTCHAs automatically, call third-party solver APIs, inject challenge
response tokens, change browser fingerprints, or bypass access
controls.

### Eval semantics

`js` can be an **expression** or a **function body**; the daemon
picks the right form with a compile-only probe, so both just work:

```sh
hwatu eval 'document.title'                # expression
hwatu eval '1+1'                           # expression
hwatu eval 'return document.title'        # function body
hwatu eval 'const n = 6*7; return n'      # statements need return
```

`await` works in both forms and a returned Promise is awaited before
the response. `undefined` maps to JSON `null`. Default timeout 15 s,
override with `timeout_ms`.

```sh
hwatu eval 'return {
  title: document.title,
  errors: [...document.querySelectorAll(".error")].map(e => e.textContent),
}'
```

### Scroll semantics

Exactly one way to say "where": `selector` (scrolled into view,
centered; disambiguate with `nth` and/or `contains`), `to_y`
(absolute pixels), or `by_pages` (viewport-heights, default 1.0,
negative = up). The response tells you what happened, so no
screenshot is needed to confirm a scroll:

```sh
hwatu scroll h2 --contains History
{"matched":{"matches":1,"tag":"h2","text":"History"},"x":0,"y":431,"max_y":8902,"at_bottom":false}
hwatu scroll --by 2           # two viewports down
hwatu scroll --to-y 0         # back to top
```

A selector that matches nothing (or `nth` past the end) is an error
that reports the match count, not a silent scroll to the wrong place.

### Snapshot: page state without pixels

`hwatu snapshot` returns the page as structured JSON: url, title, the
visible text (bounded to ~4 KB), the scroll position, and up to 120
visible **interactables** (links, buttons, inputs, selects,
role=button, contenteditable), each with a `ref` index, tag, label
text, and, where useful, `href`/`type`/`value`/`name`/`checked`:

```sh
hwatu snapshot
{"url":"http://localhost:5173/","title":"My app","text":"…",
 "interactables":[{"ref":0,"tag":"a","text":"Docs","href":"/docs"},
                  {"ref":1,"tag":"input","type":"text","name":"q","text":"Search"}],
 "scroll":{"y":0,"max_y":1180}}
```

The refs are live element handles held by the page: `hwatu click
--ref 0` or `hwatu type --ref 1 hello` target them directly, no
selector engineering. Refs go stale on navigation (you get a clear
error, not a wrong click); re-snapshot after the page changes. For
"did it render right" use `shot`; for "what is on this page and what
can I do" use `snapshot`, at a fraction of the tokens.

### Click and type

Both target elements the same way: a CSS selector (disambiguated
with `--nth` / `--contains`, like scroll) or `--ref <n>` from the
last snapshot. Both scroll the element into view first and report
what they hit.

```sh
hwatu click button --contains "Save"
{"clicked":{"matches":1,"tag":"button","text":"Save"},"url":"http://localhost:5173/"}

hwatu type 'input[name=email]' user@example.com
hwatu type 'textarea' "line one" --no-clear     # append instead of replace
hwatu type 'select[name=lang]' Korean           # picks the matching <option>
hwatu type 'input[name=q]' query --enter        # Enter; submits the form if unhandled
```

`click` dispatches a full pointer sequence (pointerdown, mousedown,
focus, pointerup, mouseup, click), so handlers listening on any of
those fire. `type` sets values through the native setter and fires
`input`/`change`, which framework-controlled inputs (React et al.)
observe. Ambiguity and misses are structured errors with match
counts, not silent no-ops.

### Console and network capture

Every window buffers, from document start: `console.*` calls,
uncaught exceptions, unhandled promise rejections, failed resource
loads, and HTTP >= 400 responses (last 500 entries). This answers
"why is the page broken" without screenshot archaeology:

```sh
hwatu console
[{"ts_ms":1784737800442,"kind":"console","level":"error","text":"something bad happened {\"code\":42}","page":"http://localhost:5173/"},
 {"ts_ms":1784737800448,"kind":"exception","level":"error","text":"unhandled rejection: lost promise\n…","page":"http://localhost:5173/"},
 {"ts_ms":1784737800449,"kind":"network","level":"error","status":404,"text":"HTTP 404","url":"http://localhost:5173/api/x","page":"http://localhost:5173/"}]

hwatu console --clear          # read and drain, so the next read is a clean diff
hwatu console --limit 10       # just the tail
```

A tight verify loop: `hwatu console --clear` before the action,
act, then `hwatu console` shows only what the action caused.

## A verification loop

Sticky targeting means the whole loop needs no ids: each command
follows the window the previous one drove.

```sh
id=$(hwatu --headless --json localhost:5173 | jq .id)   # ~14 ms
hwatu wait-load
hwatu snapshot                  # what's on the page, what can be clicked
hwatu click --ref 2             # act on it
hwatu console                   # did the page complain?
hwatu shot /tmp/after.png
hwatu close $id
```

Measured cost of that whole loop against a local dev page, medians
over 10 runs ([full data](benchmarks.md)):

| step | median |
|---|---|
| open `--headless` | 13 ms |
| `wait-load` | 50 ms |
| `eval` | 4 ms |
| `shot` (1024x768 PNG) | 142 ms |
| `close` | 8 ms |
| **total** | **216 ms** |

A full check with a screenshot costs about a fifth of a second; a
DOM-only check (skip `shot`) is ~75 ms. `eval` at 4 ms is cheap
enough to poll.

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

Daemon-based WebKitGTK browser: ~15ms window spawn, full rendering.

- When verifying frontend changes (dev server, screenshots, DOM checks),
  use hwatu instead of launching Chrome/Playwright.
- Open pages without stealing my focus: `hwatu --background <url>`
  (or `mode: background` over the socket). Use `--headless` for
  windows I should never see.
- Read page state cheaply: `hwatu snapshot` returns url, title, text,
  and indexed interactables; `hwatu click --ref <n>` / `hwatu type
  --ref <n> <text>` act on them. Selectors work too:
  `hwatu click button --contains "Save"`.
- Check for errors: `hwatu console` returns console output, uncaught
  exceptions, and failed/4xx+ network requests as JSON.
- Run JS in the page: `hwatu eval '<js expression or function body>'`
  (returns JSON), e.g. `hwatu eval 'document.title'`.
- Scroll with feedback: `hwatu scroll <selector> [--contains <text>]`
  reports what it matched and where the page landed.
- Screenshot: `hwatu shot out.png` (`--full` for the whole document).
- Verify hwatu works: `hwatu ping`.
- Docs: https://github.com/hongnoul/hwatu/blob/main/docs/agents.md
```

## Comparison

| | hwatu | headless Chrome + Playwright | Lightpanda |
|---|---|---|---|
| Spawn per check | 13-16 ms (warm) | seconds | fast |
| Rendering / screenshots | full WebKit | full Chromium | none |
| Memory | one shared engine, ~56 MB/window | ~GBs per browser | very low |
| Headed↔headless | per window, switchable live | fixed at launch | headless only |
| Human hand-off | `hwatu focus <id>` | none | none |
| Protocol | 1-line JSON over Unix socket | CDP / Playwright API | CDP subset |
| Best at | dev-loop verification | cross-browser E2E, CI | scraping at scale |

Engine caveat: hwatu renders with WebKit, end users mostly run
Chromium. For "did my change render / is the text right / did the
request fire" checks this is irrelevant; for engine-specific bugs,
keep your CI Playwright matrix.
