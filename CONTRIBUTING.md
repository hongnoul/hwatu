# Contributing to hwatu

Bug reports, patches, and packaging work are all welcome.

## Priorities

hwatu is one shared browser platform with two products. AI verification remains
first when shared-runtime priorities conflict. The tiling-WM browser has its own
accepted daily-driver roadmap rather than being frozen at hand-off quality.
Choose the relevant plan before proposing work:

- [AI verification](docs/roadmaps/verification.md)
- [Tiling-WM browser](docs/roadmaps/browser.md)
- [Shared platform](docs/roadmaps/platform.md)

Reusable behavior moves into the platform through an explicit capability and
tests. Do not make verification depend on browser-shell policy or duplicate
runtime machinery between products.

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

- Preserve the product boundary: agent-first verification, a focused tiling-WM
  browser, and no tabs. The WM does window management. Check the
  [roadmap index](docs/roadmap.md) and the relevant track before building a
  feature.
- New engine knobs go in code with correct defaults, not in a config file.
- `cargo test && cargo clippy` must pass.
- One logical change per PR.

## Packaging

Distro packaging is the most valuable non-code contribution. `packaging/`
has a PKGBUILD to start from; nixpkgs and other distros welcome.
