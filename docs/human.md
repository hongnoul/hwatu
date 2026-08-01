# hwatu for humans

The agent side is the product; the human side is a deliberately
minimal WebKit browser for tiling WMs, scoped to hand-off quality.
This page covers using hwatu as a person.

## The browser

No tabs (your WM tiles are the tabs), no chrome (a monochrome prompt
surface summoned on demand), mainstream browser keybinds
(`~/.config/hwatu/keys.conf`), built-in native ad blocking (EasyList
compiled into WebKit's content-extension engine, zero JS in the
request path), sane downloads, crash-restore sessions, TLS and
permission prompts as one-line y/n bar prompts.

If you want link hints, history completion, and password-manager
integration as a daily driver, qutebrowser is the better choice, and
we say so. Scope and non-goals: [roadmap.md](roadmap.md).

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

## Tiling WM setup

Ready-made configs: [examples/hyprland.conf](../examples/hyprland.conf)
and [examples/sway.config](../examples/sway.config). Windows carry
app_id / WM_CLASS `dev.hwatu.hwatud` for window rules; background
windows use `hwatu-background` so you can rule them onto a scratch
workspace and keep `noinitialfocus`.

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
pool. If the daemon dies uncleanly, the next start reopens every
window at its last URL.

Crates: `crates/ipc` (JSON protocol), `crates/hwatud` (daemon, GTK4 +
webkit6), `crates/hwatu` (client, no GTK linkage).
