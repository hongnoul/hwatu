# hwatud-mac: macOS daemon for hwatu

The stock daemon (`crates/hwatud`) is WebKitGTK + Wayland and cannot run on
macOS. This Swift daemon backs the unmodified `hwatu` CLI with WKWebView,
speaking the same newline-delimited-JSON protocol on the same socket path.

## Build and install

```sh
swiftc -O -o hwatud-mac Sources/main.swift -framework AppKit -framework WebKit
cp hwatud-mac ~/.local/bin/hwatud   # the CLI spawns `hwatud` from PATH
```

## Supported requests

ping, open, list, close, focus, unfocus, navigate, eval, wait_load,
screenshot, resize, get_content, snapshot, click, type, press, fill_form,
scroll, console, quit. Unsupported commands answer a clean error naming
the command.

## Headless notes

WKWebView suspends rAF/compositing for non-visible windows and reports
`document.visibilityState === "hidden"`, which stalls render-gated JS
(e.g. the Okta sign-in widget). Headless windows therefore:

- live in a ~0-alpha, click-through, cycle-ignored window kept below
  desktop icons (visible to the window server, imperceptible to the user),
- get an injected rAF shim that races native rAF against a 32 ms timer,
- spoof the Page Visibility API to always report visible.
