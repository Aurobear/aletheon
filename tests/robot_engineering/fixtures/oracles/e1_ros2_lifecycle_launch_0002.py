#!/usr/bin/env python3
"""Task-specific oracle for e1_ros2_lifecycle_launch_0002 — Lifecycle FSM."""
import importlib.util, sys, os
def main():
    ws = sys.argv[1] if len(sys.argv) > 1 else "."
    spec = importlib.util.spec_from_file_location("fsm", os.path.join(ws, "lifecycle_fsm.py"))
    mod = importlib.util.module_from_spec(spec)
    sys.modules["fsm"] = mod
    spec.loader.exec_module(mod)
    node = mod.LifecycleNode()
    if node.transition_to(mod.LifecycleState.ACTIVE):
        print("FAIL[reordering]: unconfigured->active should be denied", file=sys.stderr); return 1
    if not node.configure():
        print("FAIL[reordering]: configure should succeed", file=sys.stderr); return 1
    if not node.activate():
        print("FAIL[reordering]: activate should succeed", file=sys.stderr); return 1
    print("PASS")
    return 0
if __name__ == "__main__":
    raise SystemExit(main())
