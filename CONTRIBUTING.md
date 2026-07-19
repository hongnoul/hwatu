# Contributing to hwatu

Bug reports, patches, and packaging work are all welcome.

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

- Keep the philosophy: no tabs, no chrome, the WM does window management.
- New engine knobs go in code with correct defaults, not in a config file.
- `cargo test && cargo clippy` must pass.
- One logical change per PR.

## Packaging

Distro packaging is the most valuable non-code contribution. `packaging/`
has a PKGBUILD to start from; nixpkgs and other distros welcome.
