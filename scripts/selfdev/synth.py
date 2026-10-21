#!/usr/bin/env python3
"""Stage 3 of the closed self-development loop (see bottleneck.md).

Consumes `profile.py --json` output and emits concrete, ranked fix
candidates per bottleneck class:

  sleeps     every `sleep N` paired with what the command did next,
             plus the hwatu expect/poll replacement to write
  sequences  recurring command n-grams across the trace that should be
             compiled into one composite call or skill
  outputs    tool results large enough to bloat prefill, with the
             command that produced them

Each candidate has an estimated seconds-saved so the loop always
attacks the current #1. Verification (stage 4) is a replay: apply the
fix, rerun the task, keep it only if wall clock drops and the task
still succeeds.

Usage:
  profile.py --grep budae --json | synth.py
  profile.py --latest 10 --json | synth.py
"""

import json
import re
import sys
from collections import Counter

SLEEP_RE = re.compile(r"(?<![\w.])sleep\s+(\d+(?:\.\d+)?)")
LARGE_OUTPUT_CHARS = 4000  # ~1k tokens of prefill per turn thereafter


def norm_command(cmd):
    """Normalize a command to its shape so repeats cluster."""
    c = cmd or ""
    c = SLEEP_RE.sub("sleep N", c)
    c = re.sub(r"--id\s*\d+", "--id N", c)
    c = re.sub(r"'(?:[^'\\]|\\.){20,}'", "'<JS>'", c)   # long quoted payloads
    c = re.sub(r'"(?:[^"\\]|\\.){20,}"', '"<STR>"', c)
    c = re.sub(r"\d{3,}", "N", c)
    return c.strip()[:120]


def head_verb(cmd):
    """First meaningful token sequence, e.g. 'hwatu eval' or 'sleep N; hwatu eval'."""
    c = norm_command(cmd)
    parts = [p.strip() for p in c.split(";")]
    verbs = []
    for p in parts[:2]:
        toks = p.split()
        if not toks:
            continue
        if toks[0] in ("hwatu",) and len(toks) > 1:
            verbs.append(f"hwatu {toks[1]}")
        else:
            verbs.append(toks[0])
    return "; ".join(verbs)


def analyze(profiles):
    findings = {"sleeps": [], "sequences": [], "outputs": [], "summary": {}}
    all_calls = []
    for p in profiles:
        all_calls.extend(p.get("tool_calls", []))

    # --- sleeps ---
    total_sleep = 0.0
    for t in all_calls:
        s = t.get("sleep_s", 0)
        if s <= 0:
            continue
        total_sleep += s
        after = SLEEP_RE.split(t["command"] or "", maxsplit=1)[-1]
        after = after.lstrip("; ").replace("\n", " ")[:90]
        findings["sleeps"].append({
            "sleep_s": s,
            "command_after_sleep": after,
            "fix": "replace with condition poll: `hwatu eval` the predicate the "
                   "next command needs (2ms/op, poll at 100ms) or `hwatu expect`; "
                   "estimated recovery ~= sleep_s minus actual readiness time",
        })
    findings["sleeps"].sort(key=lambda x: -x["sleep_s"])

    # --- recurring sequences (bigrams/trigrams of command shapes) ---
    verbs = [head_verb(t["command"]) for t in all_calls if t.get("command")]
    for n in (2, 3):
        grams = Counter(tuple(verbs[i:i + n]) for i in range(len(verbs) - n + 1))
        for g, c in grams.items():
            if c >= 3 and len(set(g)) > 1:
                findings["sequences"].append({
                    "ngram": " -> ".join(g),
                    "count": c,
                    "fix": "compile into one daemon-side composite op or one "
                           "scripted eval; saves (n-1) model turns per occurrence "
                           f"~= {(n-1)*c} turns",
                    "turns_saved": (n - 1) * c,
                })
    findings["sequences"].sort(key=lambda x: -x["turns_saved"])
    findings["sequences"] = findings["sequences"][:10]

    # --- single-shape repeats (same command shape many times) ---
    shape_counts = Counter(norm_command(t["command"]) for t in all_calls if t.get("command"))
    for shape, c in shape_counts.most_common(5):
        if c >= 4:
            findings["sequences"].append({
                "ngram": f"[repeat x{c}] {shape}",
                "count": c,
                "fix": "parameterize into a loop inside one call, or a skill step",
                "turns_saved": c - 1,
            })

    # --- oversized outputs ---
    for t in all_calls:
        rc = t.get("result_chars", 0)
        if rc >= LARGE_OUTPUT_CHARS:
            findings["outputs"].append({
                "result_chars": rc,
                "command": norm_command(t["command"]),
                "fix": "shape at source: slice/filter in the eval itself before "
                       "returning; every later turn re-prefixes this output",
            })
    findings["outputs"].sort(key=lambda x: -x["result_chars"])
    findings["outputs"] = findings["outputs"][:10]

    # --- summary / ranked plan ---
    model_s = sum(p["seconds"]["model"] for p in profiles)
    turns = sum(p["n_turns"] for p in profiles)
    per_turn = model_s / turns if turns else 0
    turns_savable = sum(s["turns_saved"] for s in findings["sequences"])
    findings["summary"] = {
        "sessions": len(profiles),
        "agent_total_s": round(sum(p["seconds"]["agent_total"] for p in profiles), 1),
        "ranked_plan": [
            {"target": "sleeps", "est_saving_s": round(total_sleep * 0.8, 1),
             "action": "replace every fixed sleep with predicate polling"},
            {"target": "turn count", "est_saving_s": round(turns_savable * per_turn, 1),
             "action": f"compile top sequences into composite ops "
                       f"(~{turns_savable} turns x {per_turn:.1f}s/turn)"},
            {"target": "output size", "est_saving_s": None,
             "action": "shape large outputs at source (prefill growth, superlinear)"},
        ],
    }
    findings["summary"]["ranked_plan"].sort(
        key=lambda x: -(x["est_saving_s"] or 0))
    return findings


def render(f):
    s = f["summary"]
    print(f"analyzed {s['sessions']} session(s), agent wall clock {s['agent_total_s']}s\n")
    print("RANKED PLAN (est. seconds saved):")
    for r in s["ranked_plan"]:
        est = f"~{r['est_saving_s']}s" if r["est_saving_s"] else "superlinear"
        print(f"  {est:>12}  {r['target']:<12} {r['action']}")
    if f["sleeps"]:
        print(f"\nSLEEPS ({sum(x['sleep_s'] for x in f['sleeps']):.0f}s total):")
        for x in f["sleeps"][:10]:
            print(f"  {x['sleep_s']:>5.0f}s before: {x['command_after_sleep']}")
        print("  fix: " + f["sleeps"][0]["fix"])
    if f["sequences"]:
        print("\nRECURRING SEQUENCES (compile into composite ops/skills):")
        for x in f["sequences"][:8]:
            print(f"  x{x['count']:<3} ~{x['turns_saved']} turns  {x['ngram']}")
    if f["outputs"]:
        print("\nOVERSIZED OUTPUTS (shape at source):")
        for x in f["outputs"][:6]:
            print(f"  {x['result_chars']:>7} chars  {x['command']}")


def main():
    data = json.load(sys.stdin)
    if isinstance(data, dict):
        data = [data]
    f = analyze(data)
    if "--json" in sys.argv:
        json.dump(f, sys.stdout, indent=1)
    else:
        render(f)


if __name__ == "__main__":
    main()
