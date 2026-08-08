#!/usr/bin/env python3
"""Task-specific oracle for e1_control_numerics_0001 — PID anti-windup."""
import importlib.util, sys, os
def main():
    ws = sys.argv[1] if len(sys.argv) > 1 else "."
    spec = importlib.util.spec_from_file_location("pid", os.path.join(ws, "pid_controller.py"))
    mod = importlib.util.module_from_spec(spec)
    sys.modules["pid"] = mod
    spec.loader.exec_module(mod)
    pid = mod.PIDController(kp=2.0, ki=0.5, kd=0.1, output_min=-10.0, output_max=10.0)
    for _ in range(200):
        pid.update(50.0, 0.0, 0.01)
    # Without anti-windup: integral = 50*0.01*0.5*200 = 50.0
    # With anti-windup: integral should be clamped much lower
    if pid.integral > 15.0:
        print(f"FAIL[latency]: integral windup detected: {pid.integral:.2f}", file=sys.stderr); return 1
    print("PASS")
    return 0
if __name__ == "__main__":
    raise SystemExit(main())
