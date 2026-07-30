#!/usr/bin/env python3
"""Analyze a scroll smoothness trace (from smoothness.html).

Input: JSON {wheel: [{t, dy, dx, mode}], frames: [{t, y}]} on stdin
or as a file argument.

Reports, per wheel tick and overall:
  - px moved per tick (settled displacement)
  - time from wheel event to first scrollY change (input latency)
  - animation duration (first movement -> settle)
  - velocity profile smoothness: number of frames where scrollY jumps
    by more than 2x the local median step (jerk), and stalls (frames
    where an active animation moved 0px)
"""
import json
import statistics
import sys


def main() -> None:
    raw = open(sys.argv[1]).read() if len(sys.argv) > 1 else sys.stdin.read()
    data = json.loads(raw)
    wheel = data["wheel"]
    frames = data["frames"]
    if not wheel or not frames:
        print("empty trace")
        return

    # Frame-to-frame movement.
    moves = []  # (t, dt, dy)
    for a, b in zip(frames, frames[1:]):
        moves.append((b["t"], b["t"] - a["t"], b["y"] - a["y"]))

    total_px = frames[-1]["y"] - frames[0]["y"]
    n_ticks = len(wheel)
    print(f"wheel events: {n_ticks}  (deltaMode={wheel[0]['mode']}, "
          f"dy values: {sorted(set(w['dy'] for w in wheel))})")
    print(f"net scroll: {total_px:.0f}px  -> {total_px / n_ticks:.1f} px/tick")

    # Input latency: per wheel event, time until the next scrollY change.
    lats = []
    for w in wheel:
        for t, _dt, dy in moves:
            if t > w["t"] and dy != 0:
                lats.append(t - w["t"])
                break
    if lats:
        print(f"wheel->movement latency ms: median={statistics.median(lats):.1f} "
              f"p95={sorted(lats)[int(len(lats) * 0.95)]:.1f} max={max(lats):.1f}")

    # Movement episodes: consecutive frames with dy != 0 (allow 1-frame gaps).
    episodes = []
    cur = None
    gap = 0
    for t, dt, dy in moves:
        if dy != 0:
            if cur is None:
                cur = {"start": t - dt, "end": t, "px": dy, "steps": [dy], "stalls": 0}
            else:
                cur["end"] = t
                cur["px"] += dy
                cur["steps"].append(dy)
                cur["stalls"] += gap
            gap = 0
        elif cur is not None:
            gap += 1
            if gap > 2:
                episodes.append(cur)
                cur = None
                gap = 0
    if cur:
        episodes.append(cur)

    print(f"movement episodes: {len(episodes)}")
    for i, ep in enumerate(episodes):
        dur = ep["end"] - ep["start"]
        steps = ep["steps"]
        peak = max(abs(s) for s in steps)
        med = statistics.median(abs(s) for s in steps)
        # Jerk: frame steps that spike above 2.5x the episode median.
        jerks = sum(1 for s in steps if abs(s) > 2.5 * med) if med else 0
        print(f"  ep{i}: {ep['px']:+.0f}px over {dur:.0f}ms "
              f"({len(steps)} frames, med step {med:.1f}px, peak {peak:.1f}px, "
              f"jerky frames {jerks}, mid-anim stall frames {ep['stalls']})")

    # Frame cadence during movement.
    move_dts = [dt for _t, dt, dy in moves if dy != 0]
    if move_dts:
        med = statistics.median(move_dts)
        print(f"frame cadence while scrolling: median {med:.1f}ms "
              f"(~{1000 / med:.0f}fps), p95 {sorted(move_dts)[int(len(move_dts) * 0.95)]:.1f}ms")


if __name__ == "__main__":
    main()
