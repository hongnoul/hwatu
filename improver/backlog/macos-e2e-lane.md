# macos-e2e-lane

- **Stage:** verify
- **Evidence:** gates.sh skips the full E2E suite on Darwin (requires the
  GTK daemon or Docker; neither available on this host today). The macOS
  verification instrument has only a smoke gate (`mac.smoke.open`). The
  improver can regress mac-only behavior without noticing.
- **Rubric:** frequency 1, activation 2, fit 3, confidence 3, cost 1
  (needs a mac-daemon fixture suite, likely > 1 day). **Total 10, but cost
  < 2 so not selectable yet. Decompose:** first slice = port
  `test-agent-loop.sh`'s open/eval/screenshot/close path to a
  `scripts/test-mac-instrument.sh` honoring the mac capability matrix.
- **Acceptance check:** gates.sh runs the mac instrument suite on Darwin and
  reports pass/fail instead of only a smoke line.
