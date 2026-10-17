# dry-test-harness-preamble

- **Stage:** health (structure triage finding, gate 7)
- **Evidence:** triage 2026-10-17: 25 `scripts/test-*.sh` files duplicate the
  same private-XDG daemon-bootstrap preamble (mktemp workdir, XDG env
  exports, daemon spawn, cleanup trap). Spaghetti risk: a bootstrap fix must
  be repeated 25 times.
- **Rubric:** frequency 2 (every harness change touches it), activation 0,
  fit 2 (keeps the verification harness trustworthy), confidence 3
  (mechanically detected), cost 2 (extract `scripts/lib/daemon-harness.sh`,
  migrate scripts incrementally). **Total 9.**
- **Acceptance check:** triage preamble-duplication count decreases
  monotonically per migration PR; all migrated tests still pass on Linux.
- **Note:** Linux-host work (tests need the GTK daemon). Queue for a Linux
  session or Docker-enabled run.
