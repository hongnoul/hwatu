# hwatud-mac: macOS daemon for hwatu

The stock daemon (`crates/hwatud`) is WebKitGTK + Wayland and cannot run on
macOS. This Swift daemon backs the unmodified `hwatu` CLI with WKWebView,
speaking the same newline-delimited-JSON protocol on the same socket path.

## Build and install

```sh
swiftc -O -o hwatud-mac Sources/main.swift Sources/PageScripts.swift -framework AppKit -framework WebKit -framework JavaScriptCore
cp hwatud-mac ~/.local/bin/hwatud   # the CLI spawns `hwatud` from PATH
```

## Supported requests

ping, open, list, close, show, hide, navigate, eval, wait_load,
screenshot, resize, get_content, snapshot, click, type, press, fill_form,
scroll, console, net, expect, auth_context, challenge, upload, batch, quit. Unsupported commands answer a clean error naming
the command. `focus` and `unfocus` are accepted aliases of `show` and `hide`.

`click` applies the `irreversible_action` structured stop, `auth_context`
reports `login_required`, and `challenge --wait` returns the `challenge`
stop on timeout, using the shared `page-scripts/`. `click --trusted` and
`type --trusted` send AppKit events straight to the web view, so the page
sees `isTrusted` events in headless windows and the user's cursor and
focus do not move. `Sources/PageScripts.swift` is generated
from `page-scripts/` by `scripts/gen-page-scripts.sh`; a Rust test fails when the copy is
stale. `scripts/test-reference-skill.sh` (with `HWATUD=<path>`) passes
against this daemon. macOS is not yet a supported platform: it lacks a
signed package and `motion` (see VISION.md support criteria).

`diff` is answered by the CLI: it captures with `screenshot` and diffs
with `hwatu-pixeldiff`, the scoring code the GTK daemon uses.

`net` uses WebKit's private `_WKResourceLoadDelegate`, checked with
`responds(to:)`. If a WebKit lacks it, `net` answers an error and nothing
else changes.

## Headless notes

WKWebView suspends rAF/compositing for non-visible windows and reports
`document.visibilityState === "hidden"`, which stalls render-gated JS
(e.g. the Okta sign-in widget). Headless windows therefore:

- live in a click-through, cycle-ignored window at alpha 0.0001 and normal
  window level. 15 stacked windows composite to 0.15% opacity. A window
  below the desktop icons is treated as occluded and its page timers drop
  to about one tick per second, which stalls `expect` and
  `wait-load --until quiescent`; normal level keeps 100 ms timers on time,
- get an injected rAF shim that races native rAF against a 32 ms timer,
- spoof the Page Visibility API to always report visible.

## Single instance

Startup refuses to run when a live daemon already owns the socket, and a
watchdog exits the daemon if its socket file is removed or replaced. Both
guard against orphaned daemons whose unreachable headless windows would
otherwise accumulate on the desktop.
