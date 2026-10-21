#!/usr/bin/env python3
"""Stage 4-6 of the closed self-development loop (see bottleneck.md).

The closing edge: a budget gate over task profiles.

  verify.py baseline <name> <session.json>   record a baseline profile
  verify.py gate <name> <session.json>       compare a rerun against the
                                             baseline; exit 0 only if the
                                             objective improved (or held
                                             within tolerance) on every
                                             gated metric
  verify.py ls                               list baselines

Gated metrics: agent wall clock, model turns, tool sleep seconds,
input tokens. A promoted fix (skill, composite op, wait predicate)
must pass this gate on a replay of the same task before it lands.
After promotion, re-profile and re-run synth.py: the loop re-ranks
and attacks the new #1.
"""

import json
import os
import subprocess
import sys

STORE = os.path.expanduser("~/.jcode/selfdev-baselines")
PROFILER = os.path.join(os.path.dirname(os.path.abspath(__file__)), "profile.py")
TOLERANCE = 0.05  # allow 5% noise on "held"

GATES = [
    # (label, extractor, must_not_grow_by_more_than_tolerance)
    ("agent_wall_clock_s", lambda p: p["seconds"]["agent_total"]),
    ("model_turns", lambda p: p["n_turns"]),
    ("tool_sleep_s", lambda p: p["seconds"]["tool_sleep"]),
    ("input_tokens", lambda p: p["tokens"]["input"]),
]


def run_profiler(session):
    out = subprocess.run(
        [sys.executable, PROFILER, session, "--json"],
        capture_output=True, text=True, check=True,
    ).stdout
    return json.loads(out)[0]


def baseline_path(name):
    return os.path.join(STORE, f"{name}.json")


def cmd_baseline(name, session):
    os.makedirs(STORE, exist_ok=True)
    p = run_profiler(session)
    json.dump(p, open(baseline_path(name), "w"), indent=1)
    print(f"baseline '{name}' recorded: "
          f"{p['seconds']['agent_total']}s wall, {p['n_turns']} turns, "
          f"{p['seconds']['tool_sleep']}s sleep, {p['tokens']['input']} input tok")


def cmd_gate(name, session):
    bp = baseline_path(name)
    if not os.path.exists(bp):
        print(f"no baseline '{name}'; record one first", file=sys.stderr)
        sys.exit(2)
    base = json.load(open(bp))
    cur = run_profiler(session)
    failed = False
    print(f"gate '{name}':  (baseline -> candidate)")
    improved_any = False
    for label, get in GATES:
        b, c = get(base), get(cur)
        if b > 0 and c > b * (1 + TOLERANCE):
            verdict, failed = "FAIL (regressed)", True
        elif c < b * (1 - TOLERANCE):
            verdict, improved_any = "ok (improved)", True
        else:
            verdict = "ok (held)"
        print(f"  {label:<20} {b:>10.1f} -> {c:>10.1f}   {verdict}")
    if failed:
        print("RESULT: REJECT, a gated metric regressed")
        sys.exit(1)
    if not improved_any:
        print("RESULT: REJECT, nothing improved (no reason to promote)")
        sys.exit(1)
    print("RESULT: PROMOTE")


def cmd_ls():
    if not os.path.isdir(STORE):
        return
    for f in sorted(os.listdir(STORE)):
        p = json.load(open(os.path.join(STORE, f)))
        print(f"{f[:-5]:<24} {p['seconds']['agent_total']:>8.1f}s "
              f"{p['n_turns']:>4} turns  {os.path.basename(p['path'])}")


def main():
    a = sys.argv[1:]
    if len(a) == 3 and a[0] == "baseline":
        cmd_baseline(a[1], a[2])
    elif len(a) == 3 and a[0] == "gate":
        cmd_gate(a[1], a[2])
    elif a and a[0] == "ls":
        cmd_ls()
    else:
        print(__doc__)
        sys.exit(1)


if __name__ == "__main__":
    main()
