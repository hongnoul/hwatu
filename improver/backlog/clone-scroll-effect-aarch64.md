# clone-scroll-effect-aarch64

- **Stage:** health (lane coverage)
- **Evidence:** 2026-10-17 Linux lane: scripts/test-clone-scroll-effect.sh
  fails with `clone: eval timed out after 60000 ms` during "capturing
  rendered DOM (sweep + freeze)" against a local fixture, in the 4-core
  aarch64 colima VM with LIBGL_ALWAYS_SOFTWARE=1 and the WebKit sandbox
  disabled. All other local-fixture clone tests pass (parser fixed-point,
  scroll-tracks). Unknown whether this is pure software-rendering
  slowness or an aarch64/clone-sweep performance bug worth fixing for
  real users.
- **Rubric:** frequency 1, activation 0, fit 2, confidence 2, cost 1
  (needs profiling in the container; possibly a real perf fix). Total 6,
  not currently selectable (cost < 2).
- **Current handling:** linux-lane.sh skips it with a reasoned comment;
  it still runs on real Linux hardware and CI.
- **Next step when selected:** profile the sweep (how many scroll steps,
  per-step eval cost), compare x86 container, and either raise the eval
  budget for the clone sweep proportionally to a measured baseline or fix
  the hot path.
