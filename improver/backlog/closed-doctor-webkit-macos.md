# doctor-webkit-macos

- **Stage:** verify (activation)
- **Workflow:** cli
- **Evidence:** reproduced on this host 2026-10-17. `hwatu doctor` prints
  `FAIL webkit: WebKitGTK (libwebkitgtk-6.0) not found` and exits 1 on macOS
  while `daemon: ping ok (build mac-wkwebview)` and `render: ...captured
  pixels` both pass. The engine check is Linux-only logic running on every
  platform, violating "measured parity, not claimed parity" (VISION.md:
  report engine/platform capabilities explicitly).
- **Rubric:** frequency 2 (every macOS user), activation 3 (doctor is the
  activation event and it reports failure), fit 3 (capability honesty),
  confidence 3 (reproduced), cost 3 (hours). **Total 14.**
- **Roadmap line:** VISION.md "Measured parity, not claimed parity";
  macOS verification-instrument slice.
- **Acceptance check (must fail before implementation):**
  `scripts/improver/accept/doctor-webkit-macos.sh` — on a Darwin host with
  the daemon reachable, `hwatu doctor` exits 0 and the engine line reports
  the platform's actual engine instead of demanding WebKitGTK.
