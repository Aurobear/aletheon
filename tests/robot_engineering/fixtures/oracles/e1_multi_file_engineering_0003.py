#!/usr/bin/env python3
"""Task-specific oracle for e1_multi_file_engineering_0003 — Cross-package API fix."""
import sys, os, importlib.util
def main():
    ws = sys.argv[1] if len(sys.argv) > 1 else "."
    mp_path = os.path.join(ws, "motion_planner.py")
    init_path = os.path.join(ws, "__init__.py")
    with open(mp_path) as f:
        mp_src = f.read()
    with open(init_path) as f:
        init_src = f.read()
    errors = []
    if "compute_ik" in mp_src:
        errors.append("motion_planner.py still references compute_ik")
    if "solve_ik" not in mp_src:
        errors.append("motion_planner.py missing solve_ik")
    if "compute_ik" in init_src:
        errors.append("__init__.py still exports old symbol compute_ik")
    if errors:
        for e in errors:
            print(f"FAIL[latency]: {e}", file=sys.stderr)
        return 1
    spec = importlib.util.spec_from_file_location("kin", os.path.join(ws, "kinematics.py"))
    kin = importlib.util.module_from_spec(spec)
    sys.modules["kin"] = kin
    spec.loader.exec_module(kin)
    result = kin.solve_ik((1.0, 2.0, 0.5), [0.0, 0.0, 0.0])
    if not isinstance(result, list):
        print("FAIL[latency]: solve_ik broken", file=sys.stderr); return 1
    print("PASS")
    return 0
if __name__ == "__main__":
    raise SystemExit(main())
