# Show HN draft (ready to post)

Pre-flight: this draft assumes the launch gate in launch-checklist.md is
met. Before posting, add one line to the first comment about `hwatu mcp`
(works with Claude Code/Cursor out of the box) and link the published
head-to-head benchmark table. Those two are the difference between "neat"
and "I'll adopt this today".

Title: Show HN: Hwatu – a browser your coding agent drives over a Unix socket
URL: https://github.com/hongnoul/hwatu

First comment (post immediately after submitting):

> I built hwatu because my coding agent's verification loop was absurd:
> every "did my change render" check paid a multi-second headless-Chrome
> start and hundreds of MB, on the same laptop I was working on.
>
> hwatu splits the browser the way emacsclient/wezterm split the editor: a
> daemon (hwatud) owns WebKitGTK 6 and a prewarmed WebView pool; the client
> (hwatu) is one Unix-socket roundtrip. Measured medians: 13 ms to a
> mapped, loading window; 216 ms for a full open→wait→eval→screenshot→close
> verification pass; ~56 MB per extra window on one shared engine.
>
> The protocol is one JSON line per request: snapshot (page text +
> indexed clickables), click/type by ref or selector, eval, console and
> failed-request capture, full-page screenshots, file upload. Headless
> and headed are per-window properties, not launch flags, so
> `hwatu focus <id>` materializes an agent's live headless session as a
> real window in your tiling WM. That hand-off is the part I haven't
> seen elsewhere: agent hits a CAPTCHA or wants a human judgment call,
> the human gets the same session, the agent resumes.
>
> Honest limitations: Linux-only, needs webkitgtk-6.0, renders WebKit
> not Chromium (fine for "is the button there", not for engine-specific
> bugs), and it currently needs a display server even for headless
> windows (CI mode is on the roadmap). It also works as a minimal
> human browser for tiling WMs, but if you want vim keybindings and
> link hints, qutebrowser is the better daily driver; the human UI here
> is deliberately scoped to the hand-off.
>
> Happy to answer questions.

Timing: weekday 14:00-16:00 UTC, Tue-Thu. Do not ask anyone to upvote
(HN detects voting rings). Answer every comment in the first 3 hours.
If it flops, retry once after 2+ weeks with the tiling-WM angle:
"Show HN: A browser where windows spawn in 13ms and your WM is the tab bar".
