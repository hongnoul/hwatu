# hwatu for coding agents

hwatu is a visual verification harness for coding agents: a warm
WebKit daemon where opening, driving, and closing a real rendered
browser window costs about as much as running `ls`. This is Hwatu's
primary use case, not a side feature. Its plan of record is the
[AI verification roadmap](roadmaps/verification.md), under the shared
[portfolio roadmap](roadmap.md).

It is not a scraping browser. If you need to crawl the web at scale,
use a headless-Chrome fleet or Lightpanda. hwatu is for the inner
loop of frontend development: an agent edits code, opens the page,
checks it, and moves on, dozens of times an hour, on the same
machine the human is working on.

## Why agents like it

- **13-16 ms window spawn** from a warm daemon, measured medians
  across focused/background/headless modes
  ([benchmarks](benchmarks.md)). Verification loops spawn and
  discard windows constantly; hwatu keeps the whole loop (open,
  load, read, screenshot, close) under ~200 ms with zero setup.
- **One shared engine, zero supply chain.** N windows share one
  WebKit network process and a prewarm pool (~56 MB per extra
  window). One static binary plus the distro's webkitgtk: no Node,
  no npm package, no per-version browser download.
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

## What the agent gets

| primitive | what it answers |
|---|---|
| `snapshot` | what's on this page, what can I click (JSON, ~tokens not pixels) |
| `diff --other/--baseline` | how close are these two renders, where do they differ, as a score + regions + heatmap |
| `clone` | a self-contained local copy of a live page (rendered DOM + assets), verified against the original with a measured pixel-match report |
| `motion` | every animation as numbers: duration, delay, easing, keyframes |
| `seek` | pin all animations at time t; two shots at the same t are byte-identical |
| `expect` | assert page state in one call (polls, structured pass/fail) |
| `render --stdin` | see generated HTML rendered, no temp file, no server |
| `shot` / `shot --full` | what a user would see (real GPU-composited WebKit render) |
| `click` / `type` / `scroll` / `upload` | real pointer/input events, structured errors on misses |
| `console` | JS errors, console output, failed requests since last check |
| `net` | structured per-window request log: method, url, status, type, timing |
| `challenge` | is this a CAPTCHA / anti-bot wall, should a human take over |
| `resize` | verify responsive layouts across viewport widths |
| `focus <id>` | materialize any headless session as a real window for the human |

Ambiguity is an error with a match count, never a silent wrong click.
Refs from `snapshot` are live element handles; staleness is a clear
error, not a mystery.

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
hwatu check localhost:3000 --eval 'document.title' --shot=/tmp/c.png
                                    # one-shot: open+wait+eval+shot+close
hwatu wait-load                     # block until the load settles
hwatu wait-load --until dom         # release at DOMContentLoaded (faster)
hwatu snapshot                      # text + interactables, cheaper than a shot
hwatu expect '#status' --text ready # assert page state (polls up to 5s)
hwatu expect '#status' --text ready --watch # stream initial state + truth flips until navigation
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
| `check` | `url` or `render` (+`base?`), `eval?`, `shot?`, `shot_path?`, `full?`, `baseline?`, `tolerance?`, `heatmap?`, `viewports?`, `baseline_dir?`, `until?`, `keep?`, `timeout_ms?` | one-roundtrip verify pass: open headless, load the url (or render inline HTML directly, CLI: `hwatu render`), eval/shot/diff-vs-baseline, close; replies with everything at once. `viewports` (CLI: `--viewports 360x640,1920x1080`) sweeps the same pass at N sizes on the one window, replying with per-size results under `viewports: [{size, eval, shot, diff, pass_ms}]`; screenshots get a `-<WxH>` suffix and `baseline_dir` supplies per-size baselines `<dir>/<WxH>.png` |
| `prefetch` | `url` | start loading in a headless window and return immediately; the next `check` of the same url adopts the warm window (30 s TTL, max 3) |
| `challenge` | `id?`, `wait?`, `timeout_ms?` | detect CAPTCHA/anti-bot UI; optionally wait for manual/user resolution |
| `scroll` | `id?`, `selector?`, `nth?`, `contains?`, `to_y?`, `by_pages?` | scroll and report where it landed |
| `snapshot` | `id?` | token-cheap page state: url, title, text, indexed interactables |
| `expect` | `id?`, `selector`, `nth?`, `contains?`, `text?`, `absent?`, `visible?`, `timeout_ms?` | assert page state, polling until it holds (default 5 s); failure names what WAS found. `visible` additionally requires a real rendered element: nonzero box, no display/visibility/opacity hiding (self or ancestor), in viewport, and not covered by another element (occlusion via elementFromPoint); failure names the exact cause, e.g. what covers it |
| `click` | `id?`, `selector?`, `nth?`, `contains?`, `ref?` | click an element (real pointer events), reports what it hit |
| `type` | `id?`, `selector?`/`ref?`, `text`, `clear?`, `enter?` | fill input/textarea/select/contenteditable |
| `console` | `id?`, `clear?`, `limit?` | read the console/error/network capture buffer |
| `net` | `id?`, `clear?`, `limit?` | structured per-window network request log (method, final url, HTTP status, resource type inferred from response MIME, start_ms offset from navigation start, duration_ms); bounded 500-entry ring buffer, `clear` empties it |
| `upload` | `id?`, `selector`, `path` | set a file input's files from disk |
| `clock` | `id?`, `action` (`pause`/`resume`/`step`/`set`/`seed`/`status`), `ms?`, `seed?` | control the page's virtual clock: freeze, step, or scrub every time source the page can read |
| `motion` | `id?`, `observe?`, `observe_ms?` | declared animation inventory (CSS/WAAPI/CSSOM); with `observe`, also samples the live page under virtual time and fits models to script-driven motion (velocity, period, easing, r²) |
| `ping` | | health check; returns `{build, version}` and the CLI warns when the running daemon's build differs from the client's (restart with `hwatu quit && hwatu ping`) |

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
picks the right form with a daemon-side parse (nothing but the code
that actually runs ever reaches the page), so both just work:

```sh
hwatu eval 'document.title'                # expression
hwatu eval '1+1'                           # expression
hwatu eval 'return document.title'        # function body
hwatu eval 'const n = 6*7; return n'      # statements need return
```

`await` works in both forms and a returned Promise is awaited before
the response. `undefined` maps to JSON `null`. Default timeout 15 s,
override with `timeout_ms`.

If the page **navigates while the script runs** (a click handler that
follows a link, `location =`, form submit), the document's JS context
is destroyed and the eval can never resolve. Instead of a silent
`null` or a full timeout, the daemon replies immediately with an
error naming the destination URL; `wait-load` then re-syncs you with
the new document. For `click`, `type --enter`, and `challenge --wait`
the same navigation is treated as success: the reply waits for the
load to finish and returns `{"navigated": true, "url": ...}`.

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

### Virtual time: `hwatu clock`

`hwatu seek` pins declarative animation (CSS/WAAPI) by pausing it and
setting `currentTime`. Script-driven motion has no such handle: a
`requestAnimationFrame` loop that integrates timestamp deltas (the
classic marquee/carousel/physics pattern) sails straight through a
seek. `hwatu clock` fixes that by putting *every clock the page can
read* behind one controllable virtual timeline: `performance.now`,
`Date.now`, `new Date()`/`Date()`, `setTimeout`/`setInterval`,
`requestAnimationFrame` (a
user script wraps them at document start, before any page code runs),
with CSS/WAAPI `currentTime` driven from the same clock.

```sh
hwatu clock pause        # freeze: rAF stops, timers stop, now() stops
hwatu clock step 1000    # advance exactly 1000 virtual ms (60fps ticks:
                         #   due timers fire, one rAF batch per tick)
hwatu clock set 5000     # step to absolute virtual time 5000 ms
hwatu clock resume       # back to real time, monotonic
hwatu clock seed 42      # Math.random -> seeded deterministic PRNG
hwatu clock status       # {installed, paused, virtual_ms, pending_*, seed}
```

Until the first `pause`/`step`/`set` the clock is dormant passthrough:
timers delegate 1:1 to native timers, virtual time equals real time,
and pages behave natively. After `pause`, equal steps give equal
frames: two `hwatu shot`s at the same virtual time are byte-identical,
so animated pages become diffable stills even mid-flight. In headless
windows, where the engine never grants rendering opportunities,
`step` is also what *drives* rAF and IntersectionObserver callbacks,
so visibility-gated animation runs at all.

Caveats, in the same spirit as the eval-navigation note: the clock
controls time *after* page scripts start reading it, not the page's
load. Two loads of the same URL reach `clock pause` at slightly
different real moments (network, decode), so `set 5000` on two fresh
loads is not pixel-reproducible unless you pause before the motion
starts and step from there; within one loaded page, determinism is
exact. The harness's own commands (`expect` polling, `challenge
--wait`, click settle delays) run on the native clock, so a paused
page never deadlocks the tool driving it. Pages that captured
`performance.now` into a closure before the wrapper ran (impossible
for normal loads, possible for pages loaded by a pre-clock daemon
build) report `installed: false` errors; reload the page.

`Math.random` is the one visible entropy source the clock does not
cover, so `clock seed <u64>` replaces it with a deterministic PRNG
(mulberry32). The seed applies to the current page immediately and,
via a document-start script, to every future load in that window, so
same seed + same virtual timeline (which fixes the *order* of
`Math.random()` calls) gives identical sequences across loads.
Without a seed, pages keep native `Math.random`.

### Observed motion: `hwatu motion --observe`

`hwatu motion` reads the page's *declared* animation inventory
(CSS/WAAPI/CSSOM). Script-driven motion (the rAF marquee, a canvas
container repositioned by JS, a physics tween) is invisible to all
of it. `--observe` closes the gap by watching the live page and
**fitting models, not capturing frames**:

```sh
hwatu motion --observe --ms 2500
# ... declared inventory as before, plus:
# "observed": [{"target":"ul.logo-carousel__marquee","property":"transform",
#               "axis":"x","model":"linear","velocity_px_s":-29.99,
#               "period_s":103.23,"phase_s":103.15,"fit_r2":1.0,
#               "source":"observed"}],
# "observed_meta": {"frames":150,"window_ms":2500,"wrap_hunt_ms":100800,
#                   "virtual_time":true}
```

A sampler injected into the page finds moving elements
(MutationObserver for style writers + a two-frame rect diff, topmost
movers only), samples `getBoundingClientRect` once per frame over the
window, and then *wrap-hunts*: fast-forwards time in coarse chunks
until looping tracks jump, pinning loop periods that are minutes long
without waiting minutes. The daemon fits each position series and
reports per track: `linear` (robust velocity in px/s, immune to
loop-wrap outliers; loop `period_s`/`phase_s` when evidence exists),
`periodic` (oscillation period via autocorrelation), or `bezier`
(one-shot move: duration, distance, fitted `cubic-bezier` easing).
Every fit carries `fit_r2`; treat entries below ~0.9 as "something
moved" rather than a law of motion. Identical sibling fits (one
layout shift moving a whole column) are collapsed into one entry with
`also_targets`.

Sampling runs on the virtual clock, which is not an implementation
detail but the reason this works at all: in headless windows native
rAF **never ticks** (hidden pages get no rendering opportunities), so
any real-time observer sees a frozen page. Clock-stepped sampling
drives rAF itself with virtual timestamps, and is faster than real
time (2.5 s of animation measured in about a second, plus the wrap
hunt covering minutes of virtual time). The observation perturbs the
page's timeline (time is stepped, then resumed), so run it before or
after, not during, a `seek`-pinned screenshot comparison. The output
is token-cheap JSON: positions and fits, never pixels.

## A verification loop

For a one-shot check, `hwatu check` is the whole loop in one command
(and one daemon roundtrip): it opens a headless window, waits for the
load, runs your JS, screenshots, closes the window, and replies with
url/title/eval/shot-path/console/timings as one JSON object. It can
never leak a window, even on timeout.

```sh
hwatu check localhost:5173 --eval 'document.title' --shot=/tmp/after.png
hwatu check localhost:5173 --until dom      # don't wait for slow images
hwatu check localhost:5173 --keep           # keep the window, returns id
hwatu check localhost:5173 --baseline /tmp/before.png --heatmap /tmp/heat.png
                                            # + pixel diff vs a baseline PNG
```

With `--baseline`, the reply gains a `diff` field: `match_percent`,
mismatch regions, and the envelope (engine/viewport/caveats), same
output as `hwatu diff`. One command answers both "is the DOM right"
(`--eval`) and "does it look right" (`--baseline`).

### First render: verify before a baseline exists

A new page has nothing trustworthy to pixel-diff against. Do not seed a
baseline merely because the page loaded, and do not infer visibility from
DOM existence. Establish semantic and rendered invariants first:

```sh
id=$(hwatu --headless --json localhost:5173 | jq .id)
hwatu wait-load --id "$id"
hwatu snapshot --id "$id"                         # expected controls/text exist
hwatu expect --id "$id" 'main' --visible          # rendered and unobscured
hwatu expect --id "$id" 'button' --contains Save --visible
hwatu console --id "$id"                          # no runtime/load failures
hwatu shot --id "$id" /tmp/baseline.png            # seed only after checks pass
hwatu close "$id"
```

`expect --visible` records the current scroll position, temporarily scrolls a
fully off-screen match into view, hit-tests its center and four inset corners,
and restores the original position. After a scroll inspection completes, it
reports inspection-induced document or target-geometry changes instead of
trusting a layout created by the check itself. Effective opacity is calculated
through the ancestor chain, and two matching samples are required so a
transitional frame cannot false-pass.
Requiring every hit-test sample to resolve to the element or its subtree also
catches partial overlaps such as a sticky header over the top edge. On failure,
the message identifies the instability, sampled point, or covering element.
Once the first render is approved, use that screenshot as the baseline for
subsequent `check --baseline` or `diff --baseline` passes.

Recommended agent policy: **snapshot for structure, expect for invariants,
console for runtime health, screenshot for human evidence, diff for
regressions**. These layers complement each other; no single layer is proof.

### Speculative pre-render: pay the load while you think

`hwatu prefetch <url>` starts loading the page in a headless window
and returns immediately. The next `check` of the same URL adopts the
warm window instead of navigating, reporting `"prefetched": true` and
`load_ms` near 0. Fire it right after writing a file, then compose
your check; by the time you run it, the render is done (measured
medians on a local fixture: check 84 ms, prefetched check 1 ms; see
[benchmarks.md](benchmarks.md)).

```sh
hwatu prefetch localhost:5173     # returns immediately, page loads in background
# ... agent formulates the verification ...
hwatu check localhost:5173 --eval '...' --shot   # adopts the warm window, ~5 ms
```

Unclaimed prefetches expire after 30 s (the page would be stale
anyway); at most 3 are held at once. A prefetch is never required
for correctness: a check with no matching prefetch just loads
normally.

For multi-step interaction, sticky targeting means the loop needs no
ids: each command follows the window the previous one drove.

```sh
id=$(hwatu --headless --json localhost:5173 | jq .id)   # ~14 ms
hwatu wait-load
hwatu snapshot                  # what's on the page, what can be clicked
hwatu click --ref 2             # act on it
hwatu expect '#status' --text Saved --visible  # did the intended effect happen?
hwatu console                   # supplementary runtime/request diagnostics
hwatu shot /tmp/after.png
hwatu close $id
```

Every state-changing action needs a post-action assertion. A dispatched click
does not prove that its handler existed or that the application responded, and
a clean console cannot reveal a handler that was never attached. Verify the
specific effect: a DOM or class change, navigation, request-driven result,
element disappearance, or a value that survives reload. `console` is
supplementary diagnostic evidence, never the success condition by itself.

Measured cost of that whole loop against a local dev page, medians
over 10 runs ([full data](benchmarks.md)):

| step | median |
|---|---|
| open `--headless` | 9 ms |
| `wait-load` | 49 ms |
| `eval` | 2 ms |
| `shot` (1024x768 PNG) | 15 ms |
| `close` | 6 ms |
| **total, 5 separate commands** | **87 ms** |
| **total, one `hwatu check`** | **35-39 ms** |

A full check with a screenshot costs ~35-39 ms as one command
(window recycling skips construction), faster than the same pass
through a warm in-process Playwright connection (82 ms), and ~9x
faster than warm-server Playwright driven the same service shape
(341 ms).
`eval` at 2 ms is cheap enough to poll.

`wait-load` (and `goto`, and `check`) default to the full settle:
every subresource loaded. Real pages drag that out with slow images
and third-party scripts; when the check only reads the DOM, pass
`--until dom` to release at `DOMContentLoaded` instead (a fixture
with one 800 ms-slow image: 68 ms vs 1581 ms, same DOM either way;
see [benchmarks.md](benchmarks.md)).

If a check looks wrong and you want the human to see it:

```sh
hwatu focus $id    # window appears in their WM, session intact
```

## Window modes

New visible windows request one third of the first monitor's width by
default. A personal installation can override that initial size without
changing the upstream default by adding a fractional value between 0 and 1
to `~/.config/hwatu/config.json`:

```json
{"preferred_width": 0.25}
```

- **normal**: map + request focus. What a human asked for.
- **background**: mapped, rendered, present in the WM layout, but
  no activation request: focus stays where the user has it. Pair
  with `--app-id` and a WM rule to keep these off the current
  workspace entirely (e.g. sway `assign [app_id="agent"] workspace 9`).
- **headless**: never mapped; invisible to the WM. The toplevel is
  realized but not shown, with a synthetic 1024x768 allocation so
  pages lay out at a real viewport size. `eval`, `goto`, `upload`,
  and `shot` all work. Headless windows are excluded from crash-restore
  session snapshots (they belong to the agent's loop, not the user's
  browsing).

Note on displays: `hwatud` is a GTK app and needs a display
connection, even for headless windows. Headless mode removes
focus/WM noise on a desktop; with no display at all (CI, a bare ssh
login) the daemon enters display-free mode and hosts its own child
Wayland compositor (cage/labwc/sway on the wlroots headless
backend). Viewport sizing is exact there too: `resize` measures what
the page actually sees and corrects the allocation, so a child
compositor that eats chrome rows cannot leak extra pixel rows into a
shot.

On a bare ssh login there is usually no D-Bus session bus, and
WebKit's portal lookups can then stall the first load. Start the
daemon (or a test script) under one: `dbus-run-session -- hwatud`.

Note on DPR: GTK derives surface scale from the monitors even for
unmapped headless windows, so a fractional-scale Wayland output can
leak an unexpected devicePixelRatio into verification shots. Set
`HWATU_DPR=<integer>` on the daemon to pin it: this forces the X11
backend (unless `GDK_BACKEND` is already set) and exports
`GDK_SCALE`, which reaches both the UI and web processes. Exact on a
clean X server (Xvfb); `resize` replies always report the dpr the
page actually sees.

## Agent integrations

### Guided setup

**Install → Detect workflow → Connect agent → Verify page → Hand off to human**

```sh
hwatu setup                                      # detect clients; change nothing
hwatu doctor                                     # dependency + rendering checks
hwatu setup --client claude --scope project --dry-run
hwatu setup --client claude --scope project
hwatu demo                                       # headless unless --focus is explicit
```

`setup` supports `claude`, `cursor`, `jcode`, and `generic` MCP workflows.
Configuration is always an explicit client choice; repeated setup is safe,
`--dry-run` previews the target and action, and `--undo` removes only hwatu's
entry while preserving unrelated client settings. Project scope is shareable;
user scope is personal. The three integration tiers remain:

1. Native socket integration for Jcode.
2. Standard stdio MCP through `hwatu mcp`.
3. Plain CLI plus project instructions for any shell-capable agent.

- **jcode** has a native hwatu backend for its `browser` tool
  (engine: `hwatu`), speaking the socket directly.
- **MCP**: `hwatu mcp` serves the Model Context Protocol over stdio,
  so Claude Code, Cursor, and every other MCP client can adopt hwatu
  with one config entry. It exposes the automation protocol as 20
  tools (`open`, `snapshot`, `expect`, `click`, `type_text`, `eval`,
  `console`, `screenshot`, `scroll`, `goto`, `wait_load`, `challenge`,
  `upload`, `motion`, `seek`, `clock`, `diff`, `focus`, `close`,
  `list_windows`);
  `open` defaults to headless, and
  id-less calls follow the last-driven window just like the CLI.

  The hwatu installer does not modify agent configuration. For Claude Code,
  register it for the current project with:

  ```sh
  claude mcp add hwatu -- hwatu mcp
  ```

  To commit the configuration for collaborators instead, add `--scope
  project`; Claude Code writes `.mcp.json` and asks each user to approve that
  project-scoped server when they next start `claude`.

  ```jsonc
  // e.g. Claude Code: .mcp.json / Cursor: mcp.json
  { "mcpServers": { "hwatu": { "command": "hwatu", "args": ["mcp"] } } }
  ```

  The daemon autostarts on the first tool call. For coding agents
  that can run shell commands, CLI-style invocation of `hwatu`
  subcommands is often even more token-efficient than MCP schemas;
  both speak to the same daemon and can be mixed freely.
- **Anything else**: the socket protocol above is the integration.

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
- After every state-changing action, assert the intended effect with
  `hwatu expect`: DOM/class change, navigation, request-driven result,
  disappearance, or persistence after reload. A successful click is not proof.
- Check for additional errors: `hwatu console` returns console output,
  uncaught exceptions, and failed/4xx+ requests as JSON. A clean console is
  supplementary evidence, not proof that an interaction worked.
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
| Verify pass w/ screenshot (warm) | 83 ms | 82 ms | n/a (no rendering) |
| Rendering / screenshots | full WebKit, real WM windows | full Chromium, offscreen | none |
| Runtime deps | one binary + distro webkitgtk | Node + package + browser download | one binary |
| Headed↔headless | per window, switchable live | fixed at launch | headless only |
| Human hand-off | `hwatu focus <id>` | none | none |
| Protocol | 1-line JSON / CLI / MCP | CDP / Playwright API | CDP subset |
| Best at | dev-loop verification + hand-off | cross-browser E2E, CI | scraping at scale |

Latency honesty: a warm Playwright server is faster on raw
milliseconds ([full head-to-head data](benchmarks.md)); both are far
below an agent's thinking time. hwatu's advantages are structural
(real windows, hand-off, no Node supply chain, token-cheap CLI), not
a stopwatch win.

Engine caveat: hwatu renders with WebKit, end users mostly run
Chromium. For "did my change render / is the text right / did the
request fire" checks this is irrelevant; for engine-specific bugs,
keep your CI Playwright matrix.

### The wide field

| | hwatu | Playwright (headless Chromium) | chrome-devtools-mcp | Percy / Chromatic / Applitools | ditto & site cloners | tterm & browser-in-IDE cockpits |
|---|---|---|---|---|---|---|
| Built for | agent inner loop on your machine | cross-browser E2E test suites | DevTools introspection for agents | CI visual regression gates | one-shot site→code generation | human watching an agent |
| Pixel verification | `diff`: score + regions + heatmap, 35-39 ms warm pass | `toHaveScreenshot` baselines (test-suite shaped) | screenshots only | mature, but cloud round-trip, priced per shot | none (never renders its own output) | none |
| Animations | read as numbers (`motion`), pin mid-flight (`seek`) | disable or fast-forward to end state | raw CDP | disabled to avoid flakes | captured at generation, verified by eyeball | none |
| Focus stealing at N agents | never; headless/background are window properties | headless: fine; headed: every window pops | fine headless | n/a (cloud) | n/a | its own pane |
| Human hand-off mid-session | `focus <id>`: same live session becomes a real WM window | impossible headless; headed costs focus-steal always | none | none | n/a | human is already watching |
| CAPTCHA / needs-human | `challenge` detects + structured wait/resume | manual workarounds | none | n/a | out of scope | human solves in-pane |
| Runtime deps | 1 MB binary + distro webkitgtk | Node + package + ~170 MB browser per version | Node + Chrome | SaaS account | Node + Playwright + service | full app |
| Interface cost for an agent | one JSON line / short CLI / MCP | client library, session objects | MCP over CDP verbosity | API + dashboard | REST/MCP job API | n/a |

Memory honesty: hwatu's resident memory is *higher* than
headless-shell because every hwatu window is a real GPU-composited,
WM-mappable surface, that's the price of hand-off, partially
reclaimed by suspending idle windows (~56 MB back per window after
120 s).
