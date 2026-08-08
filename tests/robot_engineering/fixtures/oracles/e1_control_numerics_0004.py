#!/usr/bin/env python3
"""Task-specific oracle for e1_control_numerics_0004 — IIR filter coefficient order."""
import importlib.util, sys, os
def main():
    ws = sys.argv[1] if len(sys.argv) > 1 else "."
    spec = importlib.util.spec_from_file_location("iir", os.path.join(ws, "iir_filter.py"))
    mod = importlib.util.module_from_spec(spec)
    sys.modules["iir"] = mod
    spec.loader.exec_module(mod)
    # b=[0.2, 0.8] means current sample weighted 0.2, previous 0.8
    # First sample with unit step: output should be 0.2 (b[0]*x[0])
    f = mod.IIRFilter(b=[0.2, 0.8], a=[1.0, 0.0])
    first = f.process(1.0)
    if abs(first - 0.2) > 0.01:
        print(f"FAIL[reordering]: first output {first:.3f} != 0.2 (b[0]*x[0])", file=sys.stderr); return 1
    # Steady state should converge to 1.0
    for _ in range(9):
        f.process(1.0)
    tenth = f.process(1.0)
    if not (0.9 < tenth < 1.1):
        print(f"FAIL[reordering]: steady state {tenth:.3f} not near 1.0", file=sys.stderr); return 1
    print("PASS")
    return 0
if __name__ == "__main__":
    raise SystemExit(main())
