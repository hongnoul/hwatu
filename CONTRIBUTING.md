# Contributing to hwatu

Bug reports, patches, and packaging work are all welcome.

## Priorities

hwatu is AI-first: the product is visual verification for coding
agents, and the roadmap ([docs/roadmap.md](docs/roadmap.md)) is
ordered around the agent path first. The human side is also becoming a
credible primary browser for keyboard-driven tiling-WM users because that
makes agent hand-off land in the browser the user already trusts. Human-browser
work is welcome when it satisfies the gated daily-driver plan in the roadmap;
speculative Chromium feature parity is not. Tabs, sync, a built-in password
store, and a general extensions platform remain explicit non-goals.

## Building

```sh
cargo build            # needs rust + webkitgtk-6.0 dev headers
cargo test
cargo clippy
```

Run the daemon from a checkout with `target/debug/hwatud`, then drive it with
`target/debug/hwatu <url>`.

## Reporting bugs

Include the startup line `hwatud` logs (WebKitGTK version, session type,
renderer) plus your compositor. For rendering jank, that line is essential.
The [bug report form](https://github.com/hongnoul/hwatu/issues/new?template=bug-report.yml)
prompts for the information needed to reproduce a failure.

If hwatu worked but did not fit your agent workflow, submit a short
[use report](https://github.com/hongnoul/hwatu/issues/new?template=use-report.yml).
Successful workflows are as valuable as failures because they reveal which
integration paths deserve compatibility fixtures. See the
[continuous-improvement playbook](docs/continuous-improvement.md) for how
reports become roadmap and regression work.

## Pull requests

- Keep the philosophy: agent-first, no tabs, no chrome, the WM does
  window management. Check [docs/roadmap.md](docs/roadmap.md) before
  building a feature.
- New engine knobs go in code with correct defaults, not in a config file.
- `cargo test && cargo clippy` must pass.
- One logical change per PR.

## Packaging

Distro packaging is the most valuable non-code contribution. `packaging/`
has a PKGBUILD to start from; nixpkgs and other distros welcome.
