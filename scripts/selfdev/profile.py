#!/usr/bin/env python3
"""Stage 1-2 of the closed self-development loop (see bottleneck.md).

Parses Jcode session logs (~/.jcode/sessions/*.json) and attributes
wall clock to the bottleneck classes:

  model     time between a request going out and the assistant reply
            (inference + prefill)
  tool      harness tool execution time (tool_duration_ms), split into
            sleep (explicit `sleep N` in bash commands) and exec
  user      waiting for the human (excluded from the objective)

Objective function: agent wall clock per completed task
  = model + tool_exec + tool_sleep.

Usage:
  profile.py <session.json> [...]        profile specific sessions
  profile.py --latest [N]                profile the N most recent (default 1)
  profile.py --grep PATTERN              profile sessions whose text matches
  profile.py ... --json                  machine-readable output (for synth.py)
"""

import json
import os
import re
import sys
import glob
from datetime import datetime

SESSIONS_DIR = os.path.expanduser("~/.jcode/sessions")
SLEEP_RE = re.compile(r"(?<![\w.])sleep\s+(\d+(?:\.\d+)?)")


def parse_ts(s):
    return datetime.fromisoformat(s.replace("Z", "+00:00")).timestamp()


def content_blocks(msg):
    c = msg.get("content")
    if isinstance(c, list):
        return c
    return []


def extract_sleep_seconds(cmd):
    """Total explicit sleep in one bash command string."""
    return sum(float(m) for m in SLEEP_RE.findall(cmd or ""))


def profile_session(path):
    d = json.load(open(path))
    msgs = d.get("messages", [])
    events = []  # (kind, seconds, meta)
    tool_calls = []  # {name, command, duration_ms, sleep_s, result_chars}
    pending_tools = {}  # tool_use_id -> {name, command}

    prev_ts = None
    prev_role = None
    model_s = tool_exec_s = tool_sleep_s = user_s = 0.0
    tokens_in = tokens_out = cache_read = 0
    turns = 0

    for m in msgs:
        ts = parse_ts(m["timestamp"]) if m.get("timestamp") else None
        role = m.get("role")
        display = m.get("display_role")
        blocks = content_blocks(m)
        is_tool_result = any(b.get("type") == "tool_result" for b in blocks)

        if ts is not None and prev_ts is not None:
            gap = max(0.0, ts - prev_ts)
            if role == "assistant":
                # time since the last message was handed to the model
                model_s += gap
                turns += 1
            elif is_tool_result:
                dur_ms = m.get("tool_duration_ms")
                dur = (float(dur_ms) / 1000.0) if dur_ms is not None else gap
                # attribute sleeps inside the tool run
                sleep_s = 0.0
                for b in blocks:
                    tid = b.get("tool_use_id")
                    if tid in pending_tools:
                        info = pending_tools.pop(tid)
                        sleep_s = min(dur, extract_sleep_seconds(info["command"]))
                        tool_calls.append({
                            "name": info["name"],
                            "command": info["command"],
                            "duration_ms": dur * 1000.0,
                            "sleep_s": sleep_s,
                            "result_chars": sum(len(str(bb.get("content", ""))) for bb in blocks),
                        })
                tool_sleep_s += sleep_s
                tool_exec_s += max(0.0, dur - sleep_s)
                # any gap beyond tool duration is model-side dispatch; fold into model
                model_s += max(0.0, gap - dur)
            elif role == "user" and display != "system":
                user_s += gap

        if role == "assistant":
            tu = m.get("token_usage") or {}
            tokens_in += tu.get("input_tokens", 0)
            tokens_out += tu.get("output_tokens", 0)
            cache_read += tu.get("cache_read_input_tokens", 0)
            for b in blocks:
                if b.get("type") == "tool_use":
                    inp = b.get("input") or {}
                    cmd = inp.get("command") or inp.get("script") or json.dumps(inp)[:200]
                    pending_tools[b.get("id")] = {"name": b.get("name"), "command": cmd}

        prev_ts = ts if ts is not None else prev_ts
        prev_role = role

    agent_s = model_s + tool_exec_s + tool_sleep_s
    return {
        "path": path,
        "title": d.get("title"),
        "model": d.get("model"),
        "n_messages": len(msgs),
        "n_turns": turns,
        "n_tool_calls": len(tool_calls),
        "seconds": {
            "model": round(model_s, 1),
            "tool_exec": round(tool_exec_s, 1),
            "tool_sleep": round(tool_sleep_s, 1),
            "user_wait": round(user_s, 1),
            "agent_total": round(agent_s, 1),
        },
        "tokens": {"input": tokens_in, "output": tokens_out, "cache_read": cache_read},
        "tool_calls": tool_calls,
    }


def render(p):
    s = p["seconds"]
    tot = s["agent_total"] or 1.0
    print(f"\n== {os.path.basename(p['path'])}")
    print(f"   title: {p['title']!r}  model: {p['model']}")
    print(f"   turns: {p['n_turns']}  tool calls: {p['n_tool_calls']}  "
          f"tokens in/out/cached: {p['tokens']['input']}/{p['tokens']['output']}/{p['tokens']['cache_read']}")
    print(f"   agent wall clock: {tot:.0f}s  (+{s['user_wait']:.0f}s waiting on user)")
    rows = [
        ("model (inference+prefill)", s["model"]),
        ("tool exec", s["tool_exec"]),
        ("tool sleep (explicit)", s["tool_sleep"]),
    ]
    rows.sort(key=lambda r: -r[1])
    for name, v in rows:
        bar = "#" * int(40 * v / tot)
        print(f"   {name:<28} {v:>7.1f}s {100*v/tot:>5.1f}%  {bar}")
    if p["n_turns"]:
        print(f"   per turn: model {s['model']/p['n_turns']:.1f}s")
    slow = sorted(p["tool_calls"], key=lambda t: -t["duration_ms"])[:5]
    if slow:
        print("   slowest tool calls:")
        for t in slow:
            cmd = (t["command"] or "").replace("\n", " ")[:70]
            sl = f" (sleep {t['sleep_s']:.0f}s)" if t["sleep_s"] else ""
            print(f"     {t['duration_ms']/1000:>6.1f}s{sl}  {t['name']}: {cmd}")


def pick_sessions(argv):
    if "--latest" in argv:
        i = argv.index("--latest")
        n = int(argv[i + 1]) if i + 1 < len(argv) and argv[i + 1].isdigit() else 1
        files = sorted(glob.glob(os.path.join(SESSIONS_DIR, "session_*.json")),
                       key=os.path.getmtime, reverse=True)
        return files[:n]
    if "--grep" in argv:
        pat = argv[argv.index("--grep") + 1]
        out = []
        for f in sorted(glob.glob(os.path.join(SESSIONS_DIR, "session_*.json")),
                        key=os.path.getmtime, reverse=True)[:100]:
            try:
                if re.search(pat, open(f, errors="ignore").read(), re.I):
                    out.append(f)
            except OSError:
                pass
        return out[:5]
    return [a for a in argv if a.endswith(".json") and os.path.exists(a)]


def main():
    argv = sys.argv[1:]
    as_json = "--json" in argv
    files = pick_sessions(argv)
    if not files:
        print(__doc__)
        sys.exit(1)
    profiles = [profile_session(f) for f in files]
    if as_json:
        json.dump(profiles, sys.stdout, indent=1)
    else:
        for p in profiles:
            render(p)


if __name__ == "__main__":
    main()
