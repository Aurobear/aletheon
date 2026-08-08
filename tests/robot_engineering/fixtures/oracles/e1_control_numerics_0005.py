#!/usr/bin/env python3
"""Deadband hysteresis"""
import importlib.util, sys, os
def main():
    ws = sys.argv[1] if len(sys.argv) > 1 else "."
    spec = importlib.util.spec_from_file_location("deadband", os.path.join(ws, "deadband.py"))
    mod = importlib.util.module_from_spec(spec)
    sys.modules["deadband"] = mod
    spec.loader.exec_module(mod)
    f = mod.DeadbandFilter(threshold=1.0)
    inputs = [0.9, 1.01, 0.99, 1.01, 0.99, 1.01]
    outputs = [f.update(v) for v in inputs]
    changes = sum(1 for i in range(1, len(outputs)) if (outputs[i] == 0.0) != (outputs[i-1] == 0.0))
    if changes > 2:
        print(f"FAIL[packet_loss]: too many toggles ({changes})", file=sys.stderr); return 1

    print("PASS")
    return 0
if __name__ == "__main__":
    raise SystemExit(main())
