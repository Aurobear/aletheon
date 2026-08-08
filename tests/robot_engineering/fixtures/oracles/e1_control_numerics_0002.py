#!/usr/bin/env python3
"""Task-specific oracle for e1_control_numerics_0002 — Trajectory limiter."""
import importlib.util, sys, os
def main():
    ws = sys.argv[1] if len(sys.argv) > 1 else "."
    spec = importlib.util.spec_from_file_location("tl", os.path.join(ws, "trajectory_limiter.py"))
    mod = importlib.util.module_from_spec(spec)
    sys.modules["tl"] = mod
    spec.loader.exec_module(mod)
    # Scenario: moderate step changes with tight acceleration limit
    # pos changes by 2m per step at dt=0.1 -> desired vel=20, acc to get there = large
    pos = [0.0, 2.0, 4.0, 6.0, 8.0, 10.0]
    limited = mod.limit_trajectory(pos, dt=0.1, max_vel=100.0, max_acc=5.0)
    # Check acceleration constraints
    prev_vel = 0.0
    for i in range(1, len(limited)):
        vel = (limited[i] - limited[i-1]) / 0.1
        acc = (vel - prev_vel) / 0.1
        if abs(acc) > 5.0 + 1e-6:
            print(f"FAIL[reordering]: acceleration {acc:.2f} exceeds limit 5.0 at step {i}", file=sys.stderr); return 1
        prev_vel = vel
    print("PASS")
    return 0
if __name__ == "__main__":
    raise SystemExit(main())
