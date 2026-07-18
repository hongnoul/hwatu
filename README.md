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

## Philosophy

- **No tabs.** A tab is a window. Your tiling WM is the tab manager.
- **No chrome.** The WebView is the whole window.
- **Real rendering.** Full WebKit: JS, CSS, media, WebGL, as the frontend
  intended. No custom half-engine.

## Usage

```sh
hwatu                      # open a blank window (autostarts hwatud)
hwatu example.com          # open a URL (https:// implied)
hwatu list                 # id, url, title of every window
hwatu close 2              # close window 2
hwatu quit                 # stop the daemon
```

`Ctrl+q` closes the focused window. The daemon and engine stay warm.

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
