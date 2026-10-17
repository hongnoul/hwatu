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
