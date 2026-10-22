# hwatu

[![Latest Release](https://badgen.net/github/release/hongnoul/hwatu?icon=github)](https://github.com/hongnoul/hwatu/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](LICENSE)
[![CI](https://github.com/hongnoul/hwatu/actions/workflows/ci.yml/badge.svg)](https://github.com/hongnoul/hwatu/actions/workflows/ci.yml)

A headless browser that runs on the user's machine and is driven by agents
through a CLI and MCP. Target platforms: Linux, macOS, Windows.

## Install

Linux is supported today. macOS and Windows are targets and are not yet
supported.

```sh
curl -fsSL https://raw.githubusercontent.com/hongnoul/hwatu/main/scripts/install.sh | bash
```

The installer places one binary and checks for `webkitgtk-6.0`. On Arch:
`yay -S hwatu`. From source: `cargo build --release`.

```sh
hwatu doctor            # check the install
hwatu setup             # detect Claude Code, Cursor, Jcode, or MCP clients
```

`hwatu setup --dry-run` previews changes and `--undo` reverts them. Manual
MCP configuration is one entry:

```json
{ "mcpServers": { "hwatu": { "command": "hwatu", "args": ["mcp"] } } }
```

## Quick start

```sh
hwatu localhost:3000                # open a headless window, prints its id
hwatu snapshot                      # page state as data, with interactable refs
hwatu click --ref 3
hwatu type --ref 5 "Test User"
hwatu expect '.status' --text Saved # assertion with polling
hwatu shot /tmp/after.png
hwatu show 1                        # make window 1 visible to a person
```

A sequence runs in one call. `batch` waits on page state instead of fixed
delays and returns a result per step:

```sh
hwatu batch --stdin < steps.json
```

The full command set is in [docs/agents.md](docs/agents.md).

## Skills

Skills live in the agent harness. A skill is a `SKILL.md` in the harness's
skills folder that calls `hwatu` commands. This project has no skill
library, registry, or file format. See [docs/agents.md](docs/agents.md).

## Human involvement

The browser is headless and becomes visible only through `show`. When a
person is needed, a command returns a structured stop and the agent relays
it.

- Login walls are reported as `login_required`. The agent asks the user and
  enters the value once. The session persists.
- Irreversible actions (pay, place order, send, delete) return a stop with
  `reason: "irreversible_action"` and are not performed until the agent
  retries with `--confirm` after the user confirms in the current session.
- CAPTCHAs and anti-bot challenges are handled by the agent with
  `challenge`, `shot`, `snapshot`, `click --trusted`, and `type`. If the
  challenge cannot be cleared, `challenge` returns a structured block and the
  agent can hand off through `show`.

## Documents

- [VISION.md](VISION.md)
- [Agent guide](docs/agents.md)
- [Benchmarks](docs/benchmarks.md)
- [Roadmap](docs/roadmap.md)
  - [Verification](docs/roadmaps/verification.md)
  - [Shared platform](docs/roadmaps/platform.md)
  - [Native platforms](docs/roadmaps/native-platforms.md)
- [Continuous improvement](docs/continuous-improvement.md)
- [SECURITY.md](SECURITY.md)
- [CONTRIBUTING.md](CONTRIBUTING.md)

## License

MIT. See [LICENSE](LICENSE).
