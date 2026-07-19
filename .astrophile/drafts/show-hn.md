# Show HN draft (ready to post)

Title: Show HN: Hwatu – a browser where windows spawn in 45ms, your WM is the tab bar
URL: https://github.com/hongnoul/hwatu

First comment (post immediately after submitting):

> I built hwatu because opening a browser window on my tiling WM took two
> orders of magnitude longer than opening a terminal, and the browser then
> fought my WM over window management with its own tabs.
>
> It splits the browser the way emacsclient/wezterm split the editor: a
> daemon (hwatud) owns WebKitGTK 6 and a prewarmed WebView pool; the client
> (hwatu) is one Unix-socket roundtrip to a mapped, loading window. ~45-48ms
> measured warm. No tabs, no chrome: your WM tiles the windows. Unfocused
> windows suspend to disk after 2 minutes and resume instantly from the pool.
>
> Honest limitations: Linux-only, needs webkitgtk-6.0, and if you want vim
> keybindings inside the browser you want qutebrowser, not this.
>
> There's also a small JSON automation protocol over the socket (eval,
> goto, screenshot, upload), originally so coding agents can verify web
> UIs without a headless Chrome.
>
> Happy to answer questions.

Timing: weekday 14:00-16:00 UTC, Tue-Thu. Do not ask anyone to upvote
(HN detects voting rings). Answer every comment in the first 3 hours.
If it flops, retry once after 2+ weeks with the automation-protocol angle:
"Show HN: A browser your coding agent drives over a Unix socket".
