# hwatu roadmap

Status: current as of 2026-07. This file is the plan of record; docs
and marketing should match it.

## The decision

hwatu is **AI-first**. The product is visual verification for coding
agents: a warm WebKit daemon where opening, driving, screenshotting,
and closing a real rendered page costs milliseconds, on the same
machine the human is working on.

hwatu is **not** trying to be a daily-driver browser for humans. That
market belongs to Chromium derivatives on polish and to qutebrowser on
keyboard UX, and competing there would burn months on link hints,
password fill, history, and sync for a niche of a niche. The human
side of hwatu exists to serve exactly one flow: **hand-off**. An agent
drives a headless session, hits something that needs a person (a
CAPTCHA, an ambiguous UI, a visual judgment call), runs
`hwatu focus <id>`, and the same live session materializes in the
user's tiling WM. The human acts, the agent resumes.

Corollary: hwatu should be the best tool in the world at the agent
inner loop, and merely *adequate* at being looked at and poked by a
human for a minute.

## Why this is winnable

- The measured head-to-head ([benchmarks](benchmarks.md)) says raw
  latency is NOT the moat: a warm Playwright server beats hwatu on
  milliseconds, and both are far below agent thinking time. The
  structural advantages are: real WM-mappable windows (headless-shell
  cannot map one at any price), live headed↔headless switching, zero
  Node/browser-download supply chain, and a token-shaped CLI/JSON
  interface.
- Headed/headless as a *window property*, switchable live, is
  structurally impossible for launch-time-headless tools. The human
  hand-off loop is the moat.
- The maintainer dogfoods the agent path daily (jcode native
  backend), so the feedback loop is real. The human path was not
  dogfooded, which is exactly why it stagnated.

## Priorities

### P0 — adoption surface

1. **MCP server.** ~~hwatu's best features currently have one consumer
   (jcode).~~ **Shipped:** `hwatu mcp` serves MCP over stdio (no SDK,
   no new dependencies), translating tool calls onto the socket
   protocol, which stays the source of truth. Claude Code, Cursor,
   and other MCP clients adopt hwatu with one config entry.
2. **Published head-to-head benchmark** vs Playwright and
   chrome-devtools-mcp. **Shipped:** `scripts/bench-vs-playwright.mjs`,
   results and honest analysis in [benchmarks.md](benchmarks.md). It
   found real optimization targets: screenshot encode (~90 ms of the
   warm verify pass) and load-settle latency (~50 ms behind Chromium
   on the fixture). Those are now the performance workstream.

### P1 — the agent-facing "UI" (snapshot quality)

For an agent, the JSON snapshot *is* the interface. Polish it the way
a human browser polishes rendering:

3. **Snapshot diffing.** `hwatu snapshot --diff` returns only what
   changed since the last snapshot of that window. Saves tokens in
   iterate loops.
4. **Stable refs.** Interactable refs that survive re-snapshots of an
   unchanged page, with clear staleness errors on navigation (already
   partially true; make it a documented guarantee).
5. **Assertion primitives.** `hwatu expect <selector> --contains X
   --timeout 5s` and `hwatu shot --diff baseline.png`, so a verify
   loop is single commands instead of eval scripts.

### P2 — concurrency and isolation

6. **Profiles.** `--profile <name>`: separate cookie jars/site data
   per profile, so parallel agents (or one agent testing two accounts)
   don't share sessions. One daemon, N isolated headless sessions.
7. **Display-free operation.** hwatud currently needs a Wayland/X
   session even for headless windows. A nested headless compositor
   path unlocks CI.

### P3 — the hand-off loop, productized

8. **Generalized hand-off.** `challenge` already detects CAPTCHAs;
   generalize the pattern: agent flags "needs human" with a reason,
   the window materializes with the reason in the bar, the agent
   awaits resolution. One command pair, any reason.

## The human side: frozen at hand-off quality

Kept and maintained, because the hand-off flow needs them:

- The bar (find, URL entry, y/n prompts), recovery overlays, TLS and
  permission prompts, downloads, crash-restore sessions, keybinds and
  `keys.conf`, the launcher page, adblock.

Explicitly **not planned** (churn-magnet human-browser features):

- Tabs, sync, extensions/WebExtensions, password manager integration,
  link hints, browsing history and URL completion, bookmarks,
  per-site settings UI, DRM (Widevine).

Small human papercuts (zoom, undo-close, yank-URL) are accepted as
patches if they stay inside the existing Action/Keymap machinery, but
they are not roadmap items.

## Non-goals, restated

- Not a scraping browser (Lightpanda's job).
- Not a cross-browser E2E matrix (Playwright's job; hwatu is
  WebKit-only and says so).
- Not a CAPTCHA bypass tool: `challenge` is detection and hand-off
  only, by design.
