# mac-eval-semantics-parity

- **Stage:** verify (protocol semantics)
- **Workflow:** cli/socket
- **Evidence:** found by iteration 2 implementer (kangaroo), 2026-10-17:
  `hwatud-mac` passes eval text as a raw JS function body, while the Linux
  daemon auto-returns expressions. The same `hwatu eval` call yields a value
  on Linux and `undefined` on macOS unless the caller writes `return`.
  This violates "One protocol" and "command semantics guaranteed"
  (VISION.md): semantics must not change meaning by platform.
- **Rubric:** frequency 2 (every mac eval user hits it), activation 2
  (breaks first verification scripts ported from Linux), fit 3 (protocol
  parity is the moat), confidence 3 (reproduced in test-mac-instrument.sh),
  cost 2 (wrap body in platform/macos Sources/main.swift: try expression
  completion-value semantics, or mirror Linux behavior). **Total 12.**
- **Roadmap line:** docs/agents.md "Eval semantics"; VISION.md "One
  protocol" / "Measured parity, not claimed parity".
- **Acceptance check:** scripts/test-mac-instrument.sh eval assertions pass
  WITHOUT explicit `return` (update suite to use expression form once
  fixed); Linux behavior unchanged.
- **Note:** touches platform/macos/Sources/main.swift (Swift). Verify
  against docs/agents.md eval semantics section before changing.
