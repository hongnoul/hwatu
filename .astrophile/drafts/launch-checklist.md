# hwatu launch checklist

Positioning (2026-07, see docs/roadmap.md): AI-first. Lead every channel
with agent verification (13ms spawn, 216ms verify pass, human hand-off);
the tiling-WM angle is the secondary hook for WM-specific venues.
Prioritize agent-tooling channels (MCP/agent lists, agent subs) over WM
channels.

Sequenced: each step feeds the next. Snapshot before and after every step
(`astrophile snapshot`) so you know what moved the needle.

## Phase 0: before any traffic (do all, ~1 hour)
- [ ] Demo GIF/video at the top of the README (star conversion roughly doubles)
- [ ] Repo homepage URL set (`astrophile fix --apply` does this if Pages exists)
- [ ] 6-12 topics, keyword-rich description
- [ ] Tagged release with prebuilt binary
- [ ] `astrophile audit` is clean or consciously-waived

## Phase 1: permanent surfaces (week 1, before launch spikes)
- [ ] AUR package published: `astrophile login aur` (guided key linking), then `scripts/aur-publish.sh` — package indexes are discovery
- [ ] Ecosystem wiki pages (e.g. Arch Wiki, the wiki of every WM/tool you integrate with)
- [ ] llms.txt committed (AI crawlers ingest it verbatim)

## Phase 2: launch (pick ONE channel per day, answer every comment for 3h)
- [ ] Show HN: `astrophile login hn` once, then `astrophile post hn --title "<title>" --url <repo url>` — Tue-Thu, 14:00-16:00 UTC, then paste first comment from drafts/show-hn.md
- [ ] lobste.rs (needs an invite; tag appropriately)
- [ ] Reddit: `astrophile login reddit` once, then `astrophile post reddit --sub <sub> --title "<title>" --url <url>` one sub per day (titles in drafts/reddit.md)
- [ ] If HN flops: wait 2+ weeks, rewrite the title angle, try once more (allowed)

## Phase 3: compounding (after first spike)
- [ ] Awesome-list PRs (drafts/awesome-lists.md) — most require >30 days age + traction
- [ ] Newsletter submissions (e.g. Console.dev, ecosystem newsletters)
- [ ] `astrophile geo --llm ...` monthly: are AI assistants recommending hwatu yet?

Repo: https://github.com/hongnoul/hwatu
