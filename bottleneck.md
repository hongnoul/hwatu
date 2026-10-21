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

## Closed self-development loop: driving bottlenecks to zero

A loop is "closed" when every proposed fix is gated on a measured
improvement against replayed real tasks, so the system can iterate
without human judgment in the inner loop. Six stages, each feeding the
next:

### 1. Instrument (observe)

Every agent run emits a trace: per-tool-call wall clock split into
{model think, transport, daemon, page wait, sleep}, token counts in and
out, and the full command sequence. hwatu already timestamps
daemon-side; the harness adds turn-level timing. Zero-config, always
on.

### 2. Attribute (profile)

A profiler reduces traces to the ranked table above, per run and
aggregated. The loop's objective function is explicit: total wall
clock per completed task, with turn count and token volume as
secondary metrics. Regressions and plateaus are visible, not vibes.

### 3. Synthesize (generate fixes)

Each bottleneck class has a known fix generator, run offline by an
agent against the trace corpus:

- **Turn count (#1, #2):** mine recurring call sequences across
  traces, cluster them, and compile each cluster into a deterministic
  script, skill, or daemon-side composite op (the `hwatu check`
  pattern, generalized). The model is then only invoked at genuine
  novelty points; everything routine becomes code. This is the main
  engine: it converts inference time into ~ms script time.
- **Sleeps (#3):** for every `sleep N` in a trace, find the condition
  that was actually being waited on (what the next command checked)
  and synthesize an `expect`/poll predicate to replace it.
- **Page waits (#4):** learn per-site readiness signals and downgrade
  `settled` to `--until dom` + predicate where traces show it's safe.
- **Context (#5):** learn per-command output shapers (what the model
  actually used from each output) and truncate at the source.
- **Transport (#6):** route through the persistent socket / MCP once,
  mechanically.

### 4. Verify (the closing edge)

Candidate fixes run against a replay harness: recorded tasks
(fixtures + real sites) re-executed with the fix applied. Promotion
gate: task still succeeds AND wall clock / turns / tokens improve.
`scripts/bench-*.sh` style budget gates catch regressions, CI-style.
No fix lands on belief.

### 5. Promote

Passing fixes become durable assets: a new skill, a new composite
verb in hwatud, a learned wait predicate in a per-site profile, a
default output shaper. They apply to all future runs automatically.

### 6. Re-measure and repeat

The profile is re-ranked after each promotion. Because the generators
target whatever is currently #1, the loop is self-steering: as sleeps
disappear, turn-compilation becomes the whole profile, and the loop
pressure moves there.

### Fixed point

The loop converges to a floor where the only remaining cost is model
inference at genuine decision points plus irreducible page time.
Everything repeatable has been compiled away. By Amdahl, the final
lever is shrinking that residual: smaller or local models for routine
decisions, and speculative parallel branches (fire the likely next
composite op before the model confirms) so inference overlaps with
page time instead of serializing behind it.

## Implementation

The loop is live in `scripts/selfdev/`:

- `profile.py` (stages 1-2): parses Jcode session logs into the
  ranked attribution {model, tool exec, tool sleep, user wait}, with
  tokens, per-turn cost, and slowest calls.
- `synth.py` (stage 3): consumes `profile.py --json`, emits ranked fix
  candidates: every sleep with its awaited condition, recurring
  command n-grams to compile into composite ops, oversized outputs to
  shape at source, each with estimated seconds saved.
- `verify.py` (stages 4-6): baseline/gate over profiles. A fix only
  promotes if a replay improves wall clock, turns, sleep, or tokens
  without regressing any of them.
- `hwatu-poll`: first promoted fix (sleeps class): predicate polling
  that replaces `sleep N && hwatu eval <pred>`, returning the moment
  the page is ready.

First real run (the Budae Jjige DoorDash session, 204 turns, 1944 s
agent wall clock) measured: model 61.5%, explicit sleeps 29.3%
(569 s), tool exec 9.2%. Synth ranked turn-compilation first
(~984 s: 168 compilable turns x 5.9 s/turn) and sleep removal second
(~456 s). `hwatu-poll` verified on the real acceptance path: against
the live DoorDash search page from the trace, content was ready and
the poll returned in 4.4 s where the session slept a fixed 20 s (4.5x
on that wait; the fixture check had shown 1.48 s vs 1.3 s readiness).
Timeout and bad-window paths exit 1 cleanly. The gate was exercised
in both directions on real profiles: a cheaper session PROMOTEs, a
session with regressed wall clock REJECTs even though other metrics
improved, and a no-change replay REJECTs for lack of improvement.

Loop cadence: run a task, `profile.py --latest | synth.py`, implement
the top candidate, replay, `verify.py gate`, promote or discard,
repeat.

## Loop iteration 1 (2026-10-21): turn-count compilation, PROMOTED

Synth ranked turn count #1 (~1635 s across the 5-session corpus).
Fix: `scripts/selfdev/hwatu-flow`, which compiles the dominant
`goto -> wait -> eval/click` n-grams into one `hwatu batch` call with
daemon-side `expect` polling between steps.

Replay of the baseline task (find Budae Jjige price on DoorDash)
through hwatu-flow, against the live site, converged after two
extractor refinements (wrong store, then review-text false positive,
both caught by the replay itself) and reproduced the baseline's
ground-truth answer: Budae Jjige, $18.50, Seoul Restaurant Boston.

Gate verdict:

| metric | baseline | hwatu-flow replay |
|---|---|---|
| agent wall clock | 1943.7 s | 21.6 s (90x) |
| model turns | 204 | 4 |
| tool sleep | 569.5 s | 0 s |
| input tokens | 71411 | 712 |

RESULT: PROMOTE. Caveat recorded honestly: the replay is a scripted
driver, so its "turns" carry no real inference latency; the fair
reading is that the flow needs ~3 tool calls and zero sleeps where
the baseline spent 204 turns and 569 s sleeping, and the end-to-end
browser work itself completed in 10.8 s. Even charging 6 s of model
time per remaining turn, the task lands near 45 s, a ~40x
improvement, dominated by exactly the two classes synth ranked first.

Hardening pass (same day): hwatu-flow rewritten from bash to python
after edge testing exposed real defects. Verified exit-code contract
(0 ok / 1 usage / 2 bad JSON / 3 mid-batch failure / 4 no hwatu), all
observed. Cross-site generality shown on Hacker News (6-step
navigate/expect/eval/click flow, complete). Edge testing also found a
genuine integration failure mode: a cross-page click destroys the JS
context of the following step; hwatu-flow now auto-detects that race,
waits for the new document, and resumes the batch from the failed
step (verified: HN login flow completes with resumes:1, typed value
lands on the new page). Budae replay re-run on the rewritten tool:
same answer, 15.7 s, gate PROMOTE again.

Next target per re-ranked plan: oversized outputs (shape at source)
and generalizing hwatu-flow into a skill the agent reaches for by
default.

## Upstream fixes landed in hwatu (2026-10-21)

The loop's findings were promoted from agent-side workarounds into the
daemon itself, so every future agent benefits with zero setup:

1. **`--until quiescent`** (`quiescent`|`quiet`|`hydrated`): settled,
   then DOM mutation quiescence (300 ms quiet window, 5 s cap) via a
   daemon-side MutationObserver. Verified on a mutation-burst fixture
   (settled read "skeleton"; quiescent read the final menu text) and on
   live DoorDash (released at 5.1 s with 60 store links present, where
   settled released at 3.3 s with hydration still incomplete and the
   original agent session slept a blind 20 s). This deletes the sleep
   class (#3, 569 s in the baseline) at the source and subsumes
   `hwatu-poll` for load-time waits.
2. **`batch` accepts `open` and `close`**: a whole journey
   (open -> wait quiescent -> eval -> ... -> close) is now ONE IPC call
   and one agent turn. The full Budae journey ran in 4 tool calls,
   24 s, zero sleeps, zero client-side polling. This attacks the top
   class (#2 turn count) in the daemon rather than in wrapper scripts.
3. **Leaf preference for `--contains`**: click/type now drop ancestor
   matches that contain another match, so `div --contains X` hits the
   actual card instead of the page root (verified on a nested fixture:
   clicked the leaf `.name` div, not the wrapper).
4. **Actionable eval error**: WebKit's opaque "Unsupported result
   type" now explains itself (non-JSON-serializable completion value;
   return plain data) instead of reading like a wedged page. Verified
   the hint fires and that the context is NOT wedged afterwards: the
   next eval succeeds, so the historical wedge was the misread error.

All 339 workspace tests pass. `scripts/selfdev/hwatu-flow` keeps its
resume logic for cross-navigation clicks; with batch-open it can now
run id-free from the first step.
