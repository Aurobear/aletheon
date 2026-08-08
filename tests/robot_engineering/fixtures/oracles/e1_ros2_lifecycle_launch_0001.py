#!/usr/bin/env python3
"""Task-specific oracle for e1_ros2_lifecycle_launch_0001 - QoS reliability."""
import importlib.util, sys, os
def main():
    ws = sys.argv[1] if len(sys.argv) > 1 else "."
    spec = importlib.util.spec_from_file_location("subscriber", os.path.join(ws, "subscriber.py"))
    mod = importlib.util.module_from_spec(spec)
    sys.modules["subscriber"] = mod
    spec.loader.exec_module(mod)
    prof = mod.build_subscription_options()
    if prof.reliability != "reliable":
        print(f"FAIL[latency]: reliability={prof.reliability}", file=sys.stderr); return 1
    if prof.history_depth < 10:
        print(f"FAIL[latency]: history_depth={prof.history_depth}", file=sys.stderr); return 1
    print(f"PASS: QoS reliable depth={prof.history_depth}")
    return 0
if __name__ == "__main__":
    raise SystemExit(main())
