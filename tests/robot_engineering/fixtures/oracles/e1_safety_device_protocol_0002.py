#!/usr/bin/env python3
"""Task-specific oracle for e1_safety_device_protocol_0002 — Safe-stop sequencer."""
import importlib.util, sys, os
def main():
    ws = sys.argv[1] if len(sys.argv) > 1 else "."
    spec = importlib.util.spec_from_file_location("ss", os.path.join(ws, "safe_stop.py"))
    mod = importlib.util.module_from_spec(spec)
    sys.modules["ss"] = mod
    spec.loader.exec_module(mod)
    seq = mod.SafeStopSequencer()
    seq.initiate_stop(50.0)
    for _ in range(600):
        seq.step()
        if seq.phase == mod.StopPhase.COMPLETE:
            break
    if not seq.is_safe():
        print(f"FAIL[reordering]: vel={seq.velocity} brake={seq.brake_engaged}", file=sys.stderr); return 1
    print("PASS")
    return 0
if __name__ == "__main__":
    raise SystemExit(main())
