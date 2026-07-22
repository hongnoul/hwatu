# hwatu launch checklist

Positioning (2026-07, see docs/roadmap.md): AI-first. Lead every channel
with agent verification (13ms spawn, 87ms verify pass, human hand-off);
the tiling-WM angle is the secondary hook for WM-specific venues.
Prioritize agent-tooling channels (MCP/agent lists, agent subs) over WM
channels.

Sequenced: each step feeds the next. Snapshot before and after every step
(`astrophile snapshot`) so you know what moved the needle.

## Launch gate (do NOT start Phase 2 before these)

The AI-first story only lands with the roadmap P0 items shipped. You get
one launch; "WebKit daemon, jcode-only consumer" is not it.
- [x] `hwatu mcp` shipped (6ce2659, v0.5.0; 18 tools as of 82fea8b) — any
      harness (Claude Code, Cursor) can adopt it
- [x] Head-to-head benchmark published (2a4586c, docs/benchmarks.md):
      spawn, RAM per session, verify-loop latency vs Playwright+Chromium.
      This table IS the launch post. Follow-up (non-blocking): add a
      chrome-devtools-mcp column and tokens-per-snapshot row.

## Phase 0: before any traffic (do all, ~1 hour)
- [ ] Demo GIF/video at the top of the README (star conversion roughly doubles).
      Show the AGENT loop: open → snapshot → click → screenshot → close in one
      terminal, with timing visible. Not a WM rice reel.
- [x] Repo homepage URL set (`astrophile fix --apply` does this if Pages exists)
- [x] 6-12 topics, keyword-rich description (AI-first: ai-agents, coding-agents,
      mcp, browser-automation, visual-verification; done 2026-07-22)
- [ ] Tagged release with prebuilt binary
- [ ] `astrophile audit` is clean or consciously-waived

## Phase 1: permanent surfaces (week 1, before launch spikes)
- [x] AUR package published with AI-first pkgdesc (updated 2026-07-22):
      `scripts/aur-publish.sh` is idempotent, rerun after each release
- [ ] Ecosystem wiki pages (Arch Wiki; MCP server registries once `hwatu mcp` ships)
- [x] llms.txt committed (AI crawlers ingest it verbatim; AI-first copy pushed 2026-07-22)

## Phase 2: launch (after the gate; pick ONE channel per day, answer every comment for 3h)
- [ ] Show HN: `astrophile login hn` once, then `astrophile post hn --title "<title>" --url <repo url>` — Tue-Thu, 14:00-16:00 UTC, then paste first comment from drafts/show-hn.md
- [ ] Reddit, agent subs FIRST (r/ClaudeCode, r/ChatGPTCoding, r/LocalLLaMA), then r/rust, then WM subs as the secondary hook: `astrophile post reddit --sub <sub> --title "<title>" --url <url>` one sub per day (titles in drafts/reddit.md)
- [ ] lobste.rs (needs an invite; tag appropriately)
- [ ] If HN flops: wait 2+ weeks, retry once with the tiling-WM title angle (allowed)

## Phase 3: compounding (after first spike)
- [ ] Awesome-list PRs (drafts/awesome-lists.md) — agent/MCP lists first; most require >30 days age + traction
- [ ] Newsletter submissions (e.g. Console.dev, ecosystem newsletters)
- [ ] `astrophile geo --llm ...` monthly: do assistants recommend hwatu for "verify frontend changes as an agent" prompts yet?

Repo: https://github.com/hongnoul/hwatu
