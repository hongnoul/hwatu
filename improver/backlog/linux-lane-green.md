# linux-lane-green

- **Stage:** verify (lane health)
- **Evidence:** first Linux lane run 2026-10-17: clippy 2 dead-code errors
  from iteration 1's macOS-only onboarding helpers (ESCAPE: Linux gates
  were skipped on Darwin so iteration 1 regressed Linux clippy unnoticed);
  E2E 0/28 from WebKitGTK's bwrap sandbox needing userns the container
  lacks. Verified recipe: WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS=1,
  WEBKIT_DISABLE_DMABUF_RENDERER=1, LIBGL_ALWAYS_SOFTWARE=1, GTK_A11Y=none,
  xvfb-run -a => 25/28 (remaining: clone-scroll-effect timeout,
  clone-suite needs internet, display-free needs cage).
- **Rubric:** frequency 3 (every lane run), activation 0, fit 3 (gates
  must be trustworthy), confidence 3 (reproduced + recipe verified),
  cost 3 (hours). Total 12.
- **Tasks:** cfg-gate the two onboarding.rs helpers (+tests) for
  non-macOS; export the sandbox env in linux-lane.sh e2e branch with a
  loud comment that production sandbox path is NOT exercised (follow-up:
  userns/privileged container); treat known-failing 3 tests as explicit
  skips with reasons, not silent failures.
- **Acceptance check:** linux-lane.sh all => build+test+clippy green and
  e2e >= 25 passed with the 3 exclusions named.
