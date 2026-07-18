# hana-fuda 🎴

A daemon-based web browser for tiling window managers. Real WebKit rendering,
terminal-emulator spawn times.

## Why

Browsers conflate two things: the engine (slow to start, RAM-hungry) and the
window (what you actually ask for). hana-fuda splits them, the same way
`emacsclient`/`wezterm` do:

- **`hanad`** owns WebKitGTK 6, a prewarmed WebView pool, and all windows.
- **`hana`** is a thin client: one Unix-socket roundtrip to open a window.

Measured on the MVP: **~45 ms** from `hana <url>` to a mapped, loading window
(first-ever window pays a one-time engine/GPU init).

## Philosophy

- **No tabs.** A tab is a window. Your tiling WM is the tab manager.
- **No chrome.** The WebView is the whole window.
- **Real rendering.** Full WebKit: JS, CSS, media, WebGL, as the frontend
  intended. No custom half-engine.

## Usage

```sh
hana                      # open a blank window (autostarts hanad)
hana example.com          # open a URL (https:// implied)
hana list                 # id, url, title of every window
hana close 2              # close window 2
hana quit                 # stop the daemon
```

`Ctrl+q` closes the focused window. The daemon and engine stay warm.

## Build

Requires Rust and webkitgtk-6.0 (`pacman -S webkitgtk-6.0`,
`apt install libwebkitgtk-6.0-dev`).

```sh
cargo build --release
./target/release/hana example.com
```

## Architecture

```
hana <url>  --unix socket-->  hanad (GTK main loop)
                                ├── prewarmed WebView (adopted instantly,
                                │   next one warmed in idle time)
                                ├── window registry (id -> WebView)
                                └── WebKit: shared network process,
                                    per-site web processes
```

Crates:

- `crates/ipc` – newline-delimited JSON protocol (`Request`/`Response`)
- `crates/hanad` – daemon: GTK4 + webkit6, socket server on the GLib loop
- `crates/hana` – client: no GTK linkage, connects or spawns the daemon

## Roadmap

- [ ] Background window suspension + discard-to-disk (RAM reclaim)
- [ ] Per-window `app_id` for WM window rules
- [ ] Keyboard-first UX: URL overlay bar, link hints
- [ ] Session persistence and restore
- [ ] Profiles (separate cookie jars / web contexts)
- [ ] `hana ipc js ...` scripting

## Name

Hanafuda (花札) are Japanese flower cards: many small cards, one deck. Many
small windows, one engine.
