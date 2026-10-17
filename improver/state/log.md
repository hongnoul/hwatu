# Recursive-improver evidence log

One entry per iteration. Format: item, decision, measured delta, evidence
commands, gates run. See docs/recursive-improver.md.

## Baseline — 2026-10-17 (Darwin host)

- `scripts/improver/gates.sh`: portable gates green
  (cargo.test.portable, cargo.clippy.portable, ipc.compat,
  constitution.portable, triage.structure, mac.smoke.open 21ms).
  Skipped honestly: cargo.test.linux (no docker), e2e.suite (needs GTK
  daemon).
- Triage findings seeded to backlog: 25-script daemon-bootstrap
  duplication (dry-test-harness-preamble), mac E2E gap (macos-e2e-lane).
- Reproduced activation defect: doctor fails on macOS demanding WebKitGTK
  (doctor-webkit-macos, acceptance check failing as required).

## Iteration 1 — 2026-10-17 — doctor-webkit-macos — GREEN (a72d0f5)

- Change: platform-honest `hwatu doctor` (webkit + video probes) in
  crates/hwatu/src/onboarding.rs only.
- Measured delta: macOS activation fixture doctor exit 1 -> 0; engine line
  now reports WKWebView (WebKit.framework). Linux logic verified unchanged.
- Evidence: accept script ACCEPT-PASS; cargo test 122+27+8; clippy -D
  warnings clean; gates.sh --fast no FAIL; triage no new findings.
- Process: implementer (boar) and verifier (unicorn) were separate agents;
  coordinator re-ran the acceptance check before commit.
- Backlog: doctor-webkit-macos closed.

## Iteration 2 — 2026-10-17 — macos-e2e-lane (slice 1) — GREEN (72ffb73)

- Change: scripts/test-mac-instrument.sh (13 assertions, isolated daemon,
  focus-safety asserted) + gates.sh e2e.mac gate.
- Measured delta: mac verification coverage went from 1 smoke line to a
  13-assertion deterministic suite run by every gates pass (13/13 twice).
- Discovery: hwatud-mac eval semantics diverge from Linux (function body vs
  auto-return). Filed as backlog mac-eval-semantics-parity (score 12),
  demonstrating the loop generating its own next work item.
- Process note (meta-loop input): verifier (hedgehog) deleted an unrelated
  uncommitted backlog file while tidying the tree. Verifiers must be
  read-only outside their assigned diff; tighten the verifier prompt
  template.
- Backlog: macos-e2e-lane slice 1 closed; remaining slices stay open.

## Iteration 3 — 2026-10-17 — mac-eval-semantics-parity — GREEN (45054ce)

- Change: hwatud-mac eval source planning ported from Linux
  plan_eval_source (JavaScriptCore compile-only probe); README build line;
  instrument suite now 14 assertions.
- Measured delta: expression evals on macOS went from nil to correct
  values; one-protocol semantics restored per docs/agents.md. Acceptance
  5/5; instrument 14/14 twice; exactly-once execution proven by counter.
- Process note: implementer edited its own acceptance check (CLI flag
  order + load race). Verifier audited the diff adversarially and ruled it
  a legitimate harness fix with all 5 expectations intact. Keep this
  audit step mandatory whenever an implementer touches its check.
- Backlog: mac-eval-semantics-parity closed.

## Iteration 4 — 2026-10-17 — dry-test-harness-preamble (slice 1) — GREEN (1172a54)

- Change: scripts/lib/test-harness.sh extracted; two Darwin suites migrated
  off the inlined private-XDG preamble.
- Measured delta: triage duplicate-preamble count 26 -> 25; instrument
  14/14 twice; failure-propagation probe exit 1 (lib does not swallow
  failures). Lib shape validated against 4 Linux scripts for the future
  slice.
- Remaining: 25 Linux scripts migrate when a Linux host/docker is
  available (backlog item stays open, renamed note below).

## Iteration 5 — 2026-10-17 — macos-e2e-lane (slice 2) — GREEN (b0ada9c)

- Change: instrument suite 14 -> 33 assertions; every README-claimed
  hwatud-mac verb now covered with semantic assertions and covers: tags.
- Measured delta: e2e.mac gate coverage 13 -> 33; acceptance floor >= 20
  enforced; 3x deterministic; verifier manually reproduced 3 verb blocks.
- No defects found in the 8 new verbs (honest-capability rule exercised:
  implementer was authorized to file defects instead of faking passes;
  none needed).

## Meta-loop review 1 — 2026-10-17 (after 5 iterations)

Escape analysis: no regression reached main; every commit was gated and
coordinator re-ran acceptance before commit. No missed-fixture escapes
observed yet.

False-revert analysis: zero reverts in 5 iterations. Tolerances untouched.
Watch item: all 5 items were green on first verify, which may mean the
selector is picking only easy items (cost>=2 rule). Acceptable at
bootstrap; revisit if the backlog empties of cheap wins.

Scoring calibration: activation and confidence predicted real movement
(doctor fix and eval parity were the two highest-scored and highest-value
changes). No reweighting warranted on n=5.

Process defects found and fixes applied:
1. Verifier overstepped read-only in iteration 2 (deleted an uncommitted
   backlog file). Fix: verifier prompts now open with an explicit
   READ-ONLY clause. No recurrence in iterations 3-5.
2. Implementer edited its own acceptance check in iteration 3. Fix:
   verifier prompts now require an adversarial audit of acceptance-check
   diffs; applied in iterations 3-5. Policy: acceptance verb lists may be
   amended only by the coordinator.
3. gates.sh FAIL detection greps statuses, not free text; verified no
   false trips from "0 failed" strings (iteration 4 verifier confirmed a
   tail-pipe artifact, not a gate bug).

Harness cost: gates --fast ~2-3 min dominated by cargo; acceptable.
Full-suite runs still Linux-blocked (no docker on this host); that remains
the top structural gap (improver/backlog/dry-test-harness-preamble.md
Linux slice + e2e.suite). Next Linux-capable session should run both.

Loop state: healthy. Backlog open items: dry-test-harness Linux slices.
Candidate next items when hardware allows: Linux migration, full e2e
baseline, benchmark scoreboard wiring (bench-* not yet in gates).

## Iteration 6 — 2026-10-17 — aarch64-o-nofollow-security — GREEN (c493bf7)

- SECURITY: hardcoded O_NOFOLLOW was the x86-only value; symlink-overwrite
  protection was silently absent on all aarch64 Linux builds. All open(2)
  flag sites now use libc constants; macOS-only doctor helpers cfg-gated
  (fixes the Linux clippy escape from iteration 1).
- Found by the loop itself: standing up the Linux lane (linux-lane.sh +
  colima driver, 0b0603a) surfaced the failing unit test; investigator
  (kitten) proved the root cause with functional probes; implementer
  (poodle) and verifier (hare) closed it with cross-platform evidence
  including a compiled libc-value probe.
- Escape recorded for meta-loop: iteration 1 shipped a Linux clippy
  regression because Linux gates were honestly skipped but nothing
  enforced a Linux check before commit. Fix direction: linux-lane checks
  join gates.sh when the lane is reachable (linux-lane-green backlog).
- Also from the lane's first run: E2E 0/28 root-caused to the WebKit
  bwrap sandbox needing userns in docker; verified recipe reaches 25/28
  (linux-lane-green backlog, next item).

## Iteration 7 — 2026-10-17 — linux-lane-green — GREEN (4a0106b)

- Change: linux-lane.sh e2e branch exports the verified no-sandbox recipe
  (with explicit capability-honesty warning) and converts 3 known
  environment failures into named skips; new backlog item for the
  clone-scroll-effect timeout.
- Measured delta: Linux lane went 0/28 e2e -> 25 passed / 0 failed /
  3 reasoned skips; whole lane (build+test+clippy+e2e) now exit 0.
  Docker from the host also recovered after colima restart, so the
  gates.sh docker image gate now actually builds Linux (was skipped).
- Coordinator-implemented (harness-only change per meta-loop rules),
  verified by running the lane itself twice; test-d2 H15 flaked once
  (5/6 then 6/6), recorded as a watch item.
- Remaining open backlog: dry-test-harness Linux migration slices,
  userns container for sandboxed E2E, clone-scroll-effect-aarch64,
  bench scoreboard wiring.
