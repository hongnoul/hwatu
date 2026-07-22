# Contributing to hwatu

Bug reports, patches, and packaging work are all welcome.

## Priorities

hwatu is AI-first: the product is visual verification for coding
agents, and the roadmap ([docs/roadmap.md](docs/roadmap.md)) is
ordered around the agent path (MCP server, snapshot quality,
profiles/isolation, displayless CI). The human UI is deliberately
frozen at "receive an agent hand-off" quality: fixes there are
welcome, but new human-browser features (link hints, history,
password integration, sync, extensions) are explicit non-goals and
will be declined. Small human papercuts (zoom, undo-close, yank) are
fine if they stay inside the existing Action/Keymap machinery.

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
