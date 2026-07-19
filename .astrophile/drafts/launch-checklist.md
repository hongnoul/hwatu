# hwatu launch checklist

Sequenced: each step feeds the next. Snapshot before and after every step
(`astrophile snapshot`) so you know what moved the needle.

## Phase 0: before any traffic (do all, ~1 hour)
- [ ] Demo GIF/video at the top of the README (star conversion roughly doubles)
- [ ] Repo homepage URL set (`astrophile fix --apply` does this if Pages exists)
- [ ] 6-12 topics, keyword-rich description
- [ ] Tagged release with prebuilt binary
- [ ] `astrophile audit` is clean or consciously-waived

## Phase 1: permanent surfaces (week 1, before launch spikes)
- [ ] AUR package published (see drafts/PKGBUILD) — package indexes are discovery
- [ ] Ecosystem wiki pages (e.g. Arch Wiki, the wiki of every WM/tool you integrate with)
- [ ] llms.txt committed (AI crawlers ingest it verbatim)

## Phase 2: launch (pick ONE channel per day, answer every comment for 3h)
- [ ] Show HN (drafts/show-hn.md) — Tue-Thu, 14:00-16:00 UTC
- [ ] lobste.rs (needs an invite; tag appropriately)
- [ ] Reddit ecosystem subs, one per day (drafts/reddit.md)
- [ ] If HN flops: wait 2+ weeks, rewrite the title angle, try once more (allowed)

## Phase 3: compounding (after first spike)
- [ ] Awesome-list PRs (drafts/awesome-lists.md) — most require >30 days age + traction
- [ ] Newsletter submissions (e.g. Console.dev, ecosystem newsletters)
- [ ] `astrophile geo --llm ...` monthly: are AI assistants recommending hwatu yet?

Repo: https://github.com/hongnoul/hwatu
