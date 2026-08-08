#!/usr/bin/env python3
"""Angle conversion precision"""
import importlib.util, sys, os
def main():
    ws = sys.argv[1] if len(sys.argv) > 1 else "."
    spec = importlib.util.spec_from_file_location("angle_convert", os.path.join(ws, "angle_convert.py"))
    mod = importlib.util.module_from_spec(spec)
    sys.modules["angle_convert"] = mod
    spec.loader.exec_module(mod)
    import math
    original = math.pi / 4
    deg = mod.rad_to_deg(original)
    back = mod.deg_to_rad(deg)
    error = abs(original - back)
    if error > 0.01:
        print(f"FAIL[endianness]: round-trip error {error:.6f}", file=sys.stderr); return 1
    pi_approx = mod.deg_to_rad(180.0)
    if abs(pi_approx - math.pi) > 0.01:
        print(f"FAIL[endianness]: deg_to_rad(180)={pi_approx:.6f} != pi", file=sys.stderr); return 1

    print("PASS")
    return 0
if __name__ == "__main__":
    raise SystemExit(main())
