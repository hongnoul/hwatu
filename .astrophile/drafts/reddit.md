# Reddit drafts (ready to adapt)

Rule: one sub per day, never simultaneously. Answer every comment for the
first 3 hours. Post the repo link in a comment where the sub culture expects
that (r/unixporn), in the post body elsewhere.

## r/hyprland
Title: Browser windows that open as fast as terminals (45ms) — hwatu, a daemon-based WebKit browser
Body: what it is (daemon owns the engine, client is a socket roundtrip),
the `windowrule = workspace 3, class:mail` example with --app-id, and the
"your WM is the tab manager" philosophy. Link the repo.

## r/swaywm
Same as above with `assign [app_id="mail"] workspace 3`.

## r/unixporn (screenshot/video REQUIRED, lead with the rice)
Title: [Hyprland] every "tab" is a tile — browser windows spawn in 45ms (hwatu)
Body: screenshot of a tiled layout where several hwatu windows are just tiles.
Repo link in a comment.

## r/rust
Title: hwatu: a daemon-based WebKitGTK 6 browser in Rust (~45ms window spawn via prewarmed WebView pool)
Body: architecture focus: 3 crates (ipc / daemon / thin client with no GTK
linkage), newline-delimited JSON over a Unix socket on the GLib main loop,
discard-to-disk suspension. That sub stars implementations, not products.

## r/linux (later, only after traction elsewhere)
Title: hwatu: splitting the browser into engine-daemon + thin client, like emacsclient
