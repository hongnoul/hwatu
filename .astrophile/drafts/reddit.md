# Reddit drafts (ready to adapt)

Rule: one sub per day, never simultaneously. Answer every comment for the
first 3 hours. Post the repo link in a comment where the sub culture expects
that (r/unixporn), in the post body elsewhere.

Positioning note: hwatu is AI-first (agent verification). Lead with that
in dev-tool subs (r/rust, r/ClaudeCode, r/LocalLLaMA, r/ChatGPTCoding);
lead with the WM angle only in WM subs, and be upfront there that the
human UI is hand-off-scoped, not a qutebrowser replacement.

## r/ClaudeCode / r/ChatGPTCoding / agent-tooling subs (primary)
Title: A browser for agent verification loops: 13ms window spawn, 87ms open→eval→screenshot→close (hwatu)
Body: the verification-loop cost table vs headless Chrome, the snapshot/
click/type-by-ref protocol (one JSON line over a Unix socket, cheap for
token budgets), `hwatu mcp` for zero-config adoption in Claude Code/
Cursor (mention once shipped; it's the launch gate), and the human
hand-off: agent hits a CAPTCHA, `hwatu focus <id>` gives the human the
same live session in their WM, agent resumes. Paste-able AGENTS.md block
in docs/agents.md. Link the repo.

## r/rust
Title: hwatu: a daemon-based WebKitGTK 6 browser in Rust for AI agent verification (~13ms window spawn via prewarmed WebView pool)
Body: architecture focus: 3 crates (ipc / daemon / thin client with no GTK
linkage), newline-delimited JSON over a Unix socket on the GLib main loop,
discard-to-disk suspension, prewarm pool. That sub stars implementations,
not products.

## r/hyprland
Title: Browser windows that open as fast as terminals (13ms) — hwatu, a daemon-based WebKit browser
Body: what it is (daemon owns the engine, client is a socket roundtrip),
the `windowrule = workspace 3, class:mail` example with --app-id, and the
"your WM is the tab manager" philosophy. Mention the agent angle as the
reason it exists; be honest that the human UI is minimal by design.
Link the repo.

## r/swaywm
Same as above with `assign [app_id="mail"] workspace 3`.

## r/unixporn (screenshot/video REQUIRED, lead with the rice)
Title: [Hyprland] every "tab" is a tile — browser windows spawn in 13ms (hwatu)
Body: screenshot of a tiled layout where several hwatu windows are just tiles.
Repo link in a comment.

## r/linux (later, only after traction elsewhere)
Title: hwatu: splitting the browser into engine-daemon + thin client, like emacsclient
