# Time bottlenecks: hwatu + agent harness automation

Where the wall-clock goes when an agent drives hwatu through a harness
(e.g. Jcode calling `hwatu` via bash tool calls). Case study: a real
~30-tool-call browsing task (finding a menu item on a DoorDash-class
SPA), measured 2026-10-21 on an M-series Mac.

**Measured floor on this machine:** one full `hwatu` CLI round-trip
(shell fork + binary spawn + socket connect + daemon reply) is
~40-50 ms (`hwatu list` 49.6 ms, `hwatu eval` 41.2 ms). Daemon-side
eval itself is ~2 ms (see `docs/benchmarks.md`).

## Ranked by quantitative significance

| # | Bottleneck | Cost in the case-study run | Share |
|---|---|---|---|
| 1 | LLM inference per turn | ~150-300 s | ~70-80% |
| 2 | Serial turn count | multiplies #1 | - |
| 3 | Fixed sleeps | ~35 s | ~10-15% |
| 4 | Real page/network time | ~10-20 s | ~5% |
| 5 | Context growth (prefill) | folded into #1 | - |
| 6 | Bash + CLI process spawn | ~1.5 s | ~1% |
| 7 | Daemon IPC + WebKit eval | ~60 ms | ~0% |

### 1. LLM inference per turn (~70-80%)

Every tool observation requires a full model round-trip (prefill +
generation), typically 3-10 s. A 30-call run spends 2-5 minutes just
thinking. This dominates everything else by two orders of magnitude.

### 2. Serial turn count (architecture multiplier)

Each micro-step (eval, slice a string, check a flag, retype a query)
was its own agent turn. This is not a latency problem but an
architecture one: halving the number of turns halves item 1. Biggest
levers:

- Composite daemon-side ops (`hwatu check`: open+wait+eval+shot+close
  in one call, 39 ms vs 103 ms for the 5-command loop, and 4 fewer
  agent turns).
- One scripted `eval` that does find -> click -> verify instead of
  five separate turns.

### 3. Fixed sleeps (~10-15%)

The case-study transcript contained `sleep 20`, `sleep 5`, `sleep 4`
(x2), `sleep 2`: ~35 s of blind waiting on SPA renders. Replace with
condition polling (daemon-side `eval` is 2 ms, cheap to poll),
`hwatu expect`, or `wait-load --until dom`.

### 4. Real page/network time (~5%)

SPA hydration and network on heavy sites costs 1-3 s per navigation.
Irreducible except by waiting on the right event (`--until dom`
releases at DOMContentLoaded instead of full `settled`).

### 5. Context growth (superlinear tail on #1)

Each turn re-prefills the entire transcript. Late-session turns get
progressively slower. Mitigation already visible in the transcript:
trimming eval output with `.slice(0, 150)` before it reaches the
model. Keep tool outputs small by default.

### 6. Bash + CLI process spawn (~1%)

The "replace bash with something native" question. Each bash tool
call costs ~40-50 ms (shell fork + hwatu binary spawn + socket
connect). 30 calls ~= 1.5 s total. Alternatives:

- MCP server (`crates/hwatu/src/mcp.rs`) or Jcode's native `browser`
  backend: removes shell + binary spawn per call.
- Persistent socket client: drops per-op cost to ~2 ms.

Worth doing for cleanliness, but it saves ~1% of wall clock.

### 7. Daemon IPC + WebKit eval (~0%)

~2 ms per op. Already negligible; hwatu is not the bottleneck.

## Takeaway

Going native on tool transport saves ~1%. The meaningful fixes are
**fewer, fatter turns** (composite/scripted operations) and
**event-driven waits instead of sleeps**. The agent loop, not hwatu,
is where the time goes.
