#!/usr/bin/env python3
"""CSV stats"""
import importlib.util, sys, os
def main():
    ws = sys.argv[1] if len(sys.argv) > 1 else "."
    spec = importlib.util.spec_from_file_location("csv_stats", os.path.join(ws, "csv_stats.py"))
    mod = importlib.util.module_from_spec(spec)
    sys.modules["csv_stats"] = mod
    spec.loader.exec_module(mod)
    stats = mod.compute_stats([1.0, 2.0, 3.0, 4.0])
    if abs(stats["median"] - 2.5) > 0.001:
        print(f"FAIL[reordering]: median {stats['median']} != 2.5", file=sys.stderr); return 1
    expected_var = 5.0 / 3.0
    if abs(stats["variance"] - expected_var) > 0.01:
        print(f"FAIL[reordering]: variance {stats['variance']:.3f} != {expected_var:.3f}", file=sys.stderr); return 1

    print("PASS")
    return 0
if __name__ == "__main__":
    raise SystemExit(main())
