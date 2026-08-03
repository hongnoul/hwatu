# hwatu for humans

The agent side is the product, and through v0.6.x the human side
existed only to serve hand-off. v0.7.0 changed the ceiling: mainstream
keybindings, media-correct video playback, unified shortform controls,
and Chromium-curve scrolling make hwatu a credible **primary browser
for tiling window managers** like Hyprland, sway, and niri. This page
covers using hwatu as a person, end to end.

## The browser

No tabs (your WM tiles are the tabs), no chrome (a monochrome,
caption-style prompt bar summoned on demand), mainstream browser
keybinds, built-in native ad blocking (EasyList compiled into WebKit's
content-extension engine, zero JS in the request path), sane
downloads, crash-restore sessions, TLS and permission prompts as
one-line y/n bar prompts.

New windows open at a third of the monitor width, matching the first
stop of niri's default 1/3, 1/2, 2/3 preset-width cycle and a
comfortable reading width on 1080p-class monitors. Prefer a different
fraction? Set `"preferred_width"` in `~/.config/hwatu/config.json`
(see [Configuration](#configuration)). The launcher deals a
hanafuda card per window so windows are tellable apart at a glance.

If you want history completion and password-manager integration,
qutebrowser still does those better today (both are now on the
[browser roadmap](roadmaps/browser.md)). What Hwatu offers instead: one
warm engine for all windows (~56 MB per extra window), native ad
blocking with no extension process, and the agent hand-off loop no
other browser has. Portfolio scope and shared non-goals live in the
[roadmap index](roadmap.md).

## Keybindings

Defaults follow mainstream browser conventions. Everything is
rebindable in `~/.config/hwatu/keys.conf`, one
`action = chord[, chord...]` per line (`close = none` unbinds).

| action | default | does |
|---|---|---|
| `url_edit` | `ctrl+l` | edit the current URL (default text entry) |
| `yank_url` | `ctrl+y` | copy the current page URL |
| `find` / `find_next` / `find_prev` | `ctrl+f` / `ctrl+g` / `ctrl+shift+g` | find in page |
| `back` / `forward` | `alt+Left` / `alt+Right` | history |
| `reload` / `hard_reload` | `ctrl+r`, `F5` / `ctrl+shift+r` | reload |
| `zoom_in` / `zoom_out` / `zoom_reset` | `ctrl+plus` / `ctrl+minus` / `ctrl+0` | zoom |
| `new_window` | `ctrl+t`, `ctrl+n` | new window (your "new tab") |
| `close` | `ctrl+w`, `ctrl+q` | close window |
| `fullscreen` | `F11` | toggle fullscreen |
| `mute` | `m` (bare key) | toggle page audio, preference sticks across videos on the page |
| `command_palette` | `ctrl+k`, `ctrl+shift+p` | fuzzy action search in the bar |

Bare-key chords like `m` dispatch after the page declines the key, so
typing an `m` into a text box still works. Modified chords win over
the page, address-bar style. The launcher page lists every live
binding.

The Vim-style UI from earlier releases is gone, removed not hidden.

## Watching things

v0.7.0 made video pages behave, verified live per feature
([release notes](releases/v0.7.0.md)):

- **Unmuted autoplay by default** (WebKit's stock policy forced sites
  into muted fallback players). Opt out per run with
  `HWATU_AUTOPLAY=muted|deny` or persistently with
  `"autoplay": "muted"` in `~/.config/hwatu/config.json`.
- **Playback survives focus loss.** Switching WM windows no longer
  pauses Reels-style pages. Gate: `HWATU_FOCUS_SHIELD=0`.
- **Audio cannot outlive the window.** Closing a window kills its web
  process.
- **Blur-shield:** shortform pages backdrop the player with a
  CPU-rasterized 40px blur that collapsed rendering to ~34 fps; hwatu
  hides it, measured ~95 fps on YouTube Shorts. Gate:
  `HWATU_BLUR_SHIELD=0`.

Shortform feeds (Instagram Reels, YouTube Shorts, TikTok) share one
control scheme regardless of how each site builds its feed:

| key | action |
|---|---|
| ArrowUp / ArrowDown | snap exactly one video |
| Space | play / pause |
| ArrowRight (hold) | 2x playback, restores the prior rate on release |
| ArrowLeft | toggle the comment sheet |

Unknown pages fail open to native key behavior: a shortcut only
consumes the key when its target was actually found.

## Scrolling

Wheel and key scrolling (Arrow, PageUp/PageDown, Space) share a
Chromium-style animation curve with preserved velocity, replacing
WebKitGTK's isolated-pulse scroller. On snap feeds, keys page exactly
like a wheel tick. All paths bail on `defaultPrevented`, modifier
chords, and editable targets.

## Commands

```sh
hwatu                      # launcher (autostarts the daemon)
hwatu example.com          # open a URL
hwatu how to exit vim      # non-URLs become a web search
hwatu list                 # every window: id, url, title
hwatu adblock update       # fetch + compile EasyList/EasyPrivacy
hwatu update               # self-update
hwatu quit                 # stop the daemon
```

## Configuration

- `~/.config/hwatu/keys.conf`: keybindings (above).
- `~/.config/hwatu/config.json`: persistent policy.
  - `"autoplay": "muted"|"deny"`: the env var wins for a single run
    but vanishes on daemon restart.
  - `"preferred_width": 0.25`: initial window width as a fraction of
    the monitor width, between 0 and 1. Default is one third.
- Env-only gates, read at window creation: `HWATU_FOCUS_SHIELD=0`,
  `HWATU_BLUR_SHIELD=0`, `HWATU_DISABLE_MEDIA=1`.

## Tiling WM setup

Ready-made configs: [examples/hyprland.conf](../examples/hyprland.conf),
[examples/sway.config](../examples/sway.config), and
[examples/niri.kdl](../examples/niri.kdl). Windows carry
app_id / WM_CLASS `dev.hwatu.hwatud` for window rules; background
windows use `hwatu-background` so you can rule them onto a scratch
workspace and keep `noinitialfocus` (or niri's `open-focused false`).

The pattern is the same everywhere: bind a key to `hwatu` the way you
bind one to your terminal, and let the WM do the window management
hwatu deliberately doesn't duplicate.

## The hand-off, from your side

Your agent's windows don't exist on your desktop: no focus stolen,
no WM pollution, at any parallelism. Every invisible session is a
live window the WM simply hasn't been shown. When the agent hits a
CAPTCHA, an OAuth consent, a 2FA prompt, or a plain "does this look
right to you", it runs `hwatu focus <id>` and the session appears in
your tiler, same cookies, same scroll position, same half-filled
form. You act, you close or leave it, the agent resumes.

`challenge` is detection and hand-off only, by design: no solver
APIs, no token injection, no fingerprint games.

## Architecture

```
hwatu <url>  --unix socket-->  hwatud (GTK main loop)
                                 ├── prewarmed WebView (adopted instantly,
                                 │   next one warmed in idle time)
                                 ├── window registry (id -> WebView)
                                 └── WebKit: shared network process,
                                     per-site web processes
```

The daemon owns WebKitGTK 6 and a prewarmed WebView pool; the client
is one Unix-socket roundtrip, the way `emacsclient`/`wezterm` split
the editor. N windows share one engine (~56 MB per extra window);
unfocused windows suspend after 120 s and resume instantly from the
pool (a WebView playing audio is never suspended). If the daemon dies
uncleanly, the next start reopens every window at its last URL.

Crates: `crates/ipc` (JSON protocol), `crates/hwatud` (daemon, GTK4 +
webkit6), `crates/hwatu` (client, no GTK linkage).
