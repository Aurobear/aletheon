#!/usr/bin/env python3
"""Timestamp alignment"""
import importlib.util, sys, os
def main():
    ws = sys.argv[1] if len(sys.argv) > 1 else "."
    spec = importlib.util.spec_from_file_location("ts_align", os.path.join(ws, "ts_align.py"))
    mod = importlib.util.module_from_spec(spec)
    sys.modules["ts_align"] = mod
    spec.loader.exec_module(mod)
    samples = [(0, 1.0), (1, 2.0), (2, 3.0), (10, 11.0)]
    aligned = mod.align_to_master(samples, sensor_hz=100.0, master_start_s=1000.0)
    if abs(aligned[0][0] - 1000.0) > 0.001:
        print(f"FAIL[clock_drift]: first ts {aligned[0][0]}", file=sys.stderr); return 1
    expected_ts = 1000.0 + 10.0 / 100.0
    if abs(aligned[3][0] - expected_ts) > 0.001:
        print(f"FAIL[clock_drift]: sample 10 ts {aligned[3][0]} != {expected_ts}", file=sys.stderr); return 1

    print("PASS")
    return 0
if __name__ == "__main__":
    raise SystemExit(main())
