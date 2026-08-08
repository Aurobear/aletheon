#!/usr/bin/env python3
"""Task-specific oracle for e1_safety_device_protocol_0001 — Watchdog heartbeat."""
import importlib.util, sys, os
def main():
    ws = sys.argv[1] if len(sys.argv) > 1 else "."
    spec = importlib.util.spec_from_file_location("wd", os.path.join(ws, "watchdog.py"))
    mod = importlib.util.module_from_spec(spec)
    sys.modules["wd"] = mod
    spec.loader.exec_module(mod)
    wd = mod.Watchdog(ceiling_ms=200)
    now = 17
    for _ in range(20):
        wd.heartbeat(now)
        now += 100
    armed_after_partial = wd.evaluate(now)
    wd.heartbeat(now)
    now += 250
    armed_after_stall = wd.evaluate(now)
    if armed_after_partial:
        print("FAIL[watchdog]: partial stream falsely armed", file=sys.stderr); return 1
    if not armed_after_stall:
        print("FAIL[watchdog]: genuine stall did not arm", file=sys.stderr); return 1
    print("PASS")
    return 0
if __name__ == "__main__":
    raise SystemExit(main())
