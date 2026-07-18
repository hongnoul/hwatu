# hwatu 화투

[![Latest Release](https://badgen.net/github/release/hongnoul/hwatu?icon=github)](https://github.com/hongnoul/hwatu/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](LICENSE)
[![CI](https://github.com/hongnoul/hwatu/actions/workflows/ci.yml/badge.svg)](https://github.com/hongnoul/hwatu/actions/workflows/ci.yml)

A daemon-based web browser for tiling window managers. Real WebKit rendering,
terminal-emulator spawn times.

## Installation

```bash
curl -fsSL https://raw.githubusercontent.com/hongnoul/hwatu/main/scripts/install.sh | bash
```

Requires `webkitgtk-6.0` at runtime (the installer checks and tells you the
package for your distro). Or build from source:

```sh
cargo build --release   # needs rust + webkitgtk-6.0 dev headers
```

## Why

Browsers conflate two things: the engine (slow to start, RAM-hungry) and the
window (what you actually ask for). hwatu splits them, the same way
`emacsclient`/`wezterm` do:

- **`hwatud`** owns WebKitGTK 6, a prewarmed WebView pool, and all windows.
- **`hwatu`** is a thin client: one Unix-socket roundtrip to open a window.

Measured on the MVP: **~45 ms** from `hwatu <url>` to a mapped, loading window
(first-ever window pays a one-time engine/GPU init).

## hwatu vs surf, qutebrowser, luakit

If you want a minimal browser for a tiling window manager (Hyprland, sway, i3,
river), the usual suspects trade differently:

| | hwatu | surf | qutebrowser | luakit |
|---|---|---|---|---|
| Window spawn | ~45 ms (warm daemon) | full engine start per window | full engine start | full engine start |
| Engine | WebKitGTK 6 | WebKitGTK 2 | QtWebEngine (Chromium) | WebKitGTK 2 |
| Tabs | none, WM tiles are tabs | none | built-in | built-in |
| Keyboard-driven UI | your WM's binds | patches | first-class vim binds | lua config |
| Memory model | one shared engine, N views | one process per window | one big process | one process |

Honest take: if you want vim keybindings *inside* the browser, use qutebrowser.
If you want windows that appear as fast as terminals and a WM that does the
window management, that's what hwatu is for. surf pioneered this shape; hwatu
adds the warm daemon and a current engine.

## Philosophy

- **No tabs.** A tab is a window. Your tiling WM is the tab manager.
- **No chrome.** The WebView is the whole window.
- **Real rendering.** Full WebKit: JS, CSS, media, WebGL, as the frontend
  intended. No custom half-engine.
- **No ads.** Content blocking is built in and on by default, evaluated
  natively in WebKit's network process — zero JS, zero UI, zero spawn cost.

## Usage

```sh
hwatu                      # open your home page (autostarts hwatud)
hwatu example.com          # open a URL (https:// implied)
hwatu list                 # id, url, title of every window
hwatu close 2              # close window 2
hwatu adblock              # content-blocker status (rule count, source)
hwatu adblock off          # disable blocking (persisted; `on` re-enables)
hwatu adblock update       # fetch EasyList + EasyPrivacy, recompile
hwatu update               # self-update to the latest release
hwatu quit                 # stop the daemon
```

`Ctrl+q` closes the focused window. The daemon and engine stay warm.

## The bar

hwatu's one piece of chrome: a single-line vim-style bar at the bottom
of the window, hidden until summoned. Everything interactive lives
there, so the resting state stays chromeless.

- **Find in page**: `/` opens forward search, `?` backward. Matches
  highlight incrementally with a live count. `Enter` commits (focus
  returns to the page, `n`/`N` jump next/previous), `Esc` cancels.
  A `/` typed into a page's text box still goes to the page.
- **Permission prompts**: mic, camera, location, notifications,
  clipboard and friends appear as `example.com wants microphone
  [y/n]`. Decisions are remembered per site for the daemon's
  lifetime and apply across windows. Nothing is written to disk.
- **TLS errors**: failed certificate loads show the reason (expired,
  unknown issuer, hostname mismatch, ...). `y` adds a session
  exception for that host and reloads; `n`/`Esc` leaves the load
  stopped. Exceptions reset when the daemon exits.
- **Download status**: saved/failed notices flash briefly.

## Downloads

No dialogs, no download manager. Attachments and unrenderable MIME
types save straight to `HWATU_DOWNLOAD_DIR`, or your xdg-user-dirs
download folder, or `~/Downloads`. Name collisions get a ` (n)`
suffix. The bar flashes the destination when a download finishes.

## Ad blocking

A baseline filter list is embedded in the binary, so blocking works
offline on first run. `hwatu adblock update` upgrades to full EasyList +
EasyPrivacy (~117k compiled rules); compiled rulesets are cached, so
only the first start after a list change pays compile cost (~5 s), warm
starts load in ~0.4 s. Rules run in WebKit's content-extension engine in
the network process — the same machinery as Safari content blockers —
so there is no JavaScript in the request path and no per-window cost.

- Toggle: `hwatu adblock on|off` applies live to every open window and
  persists in `~/.config/hwatu/config.json`. `HWATU_ADBLOCK=off`
  overrides at daemon startup.
- Own filters: put ABP-syntax rules in `~/.config/hwatu/filters.txt`;
  they are appended to whatever lists are active.
- Filter kinds the engine cannot express declaratively ($csp,
  $redirect, scriptlets, procedural cosmetics) are skipped, never
  approximated, so a filter-list update can't break page loads.

## Tuning

No config file. Engine knobs are set to their correct values in code
(GPU compositing always on, async/threaded scrolling pinned), and the
only surface is environment variables read by `hwatud`:

- `HWATU_HOME` – page opened by a bare `hwatu`
  (default <https://hongnoul.github.io/hwatu/>, use `about:blank` for none).
- `HWATU_DISCARD_SECS` – seconds an unfocused window keeps its live
  WebView before being suspended to save RAM (default 120, 0 disables).
- `HWATU_DOWNLOAD_DIR` – where downloads land (default: xdg-user-dirs
  download folder, falling back to `~/Downloads`).
- `HWATU_WEBKIT_FEATURES=Ident:on,Other:off` – flip individual WebKit
  runtime features on odd hardware. Unknown identifiers are ignored.
- Standard `WEBKIT_*` / `GSK_RENDERER` vars pass through untouched.

Scrolling smoothness scales with your distro's WebKitGTK: 2.46+ paints
with Skia on the GPU and is markedly smoother. `hwatud` logs its
WebKitGTK version, session type, and renderer at startup; include that
line in any jank report.

## Architecture

```
hwatu <url>  --unix socket-->  hwatud (GTK main loop)
                                 ├── prewarmed WebView (adopted instantly,
                                 │   next one warmed in idle time)
                                 ├── window registry (id -> WebView)
                                 └── WebKit: shared network process,
                                     per-site web processes
```

Crates:

- `crates/ipc` – newline-delimited JSON protocol (`Request`/`Response`)
- `crates/hwatud` – daemon: GTK4 + webkit6, socket server on the GLib loop
- `crates/hwatu` – client: no GTK linkage, connects or spawns the daemon

## Roadmap

- [ ] Background window suspension + discard-to-disk (RAM reclaim)
- [ ] Per-window `app_id` for WM window rules
- [ ] Keyboard-first UX: URL overlay bar, link hints
- [ ] Session persistence and restore
- [ ] Profiles (separate cookie jars / web contexts)
- [ ] `hwatu ipc js ...` scripting

## Name

Hwatu (화투) are Korean flower cards: many small cards, one deck. Many small
windows, one engine.
