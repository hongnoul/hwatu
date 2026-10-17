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
