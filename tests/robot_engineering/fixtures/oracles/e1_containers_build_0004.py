#!/usr/bin/env python3
"""Build flags injection"""
import importlib.util, sys, os
def main():
    ws = sys.argv[1] if len(sys.argv) > 1 else "."
    spec = importlib.util.spec_from_file_location("build_flags", os.path.join(ws, "build_flags.py"))
    mod = importlib.util.module_from_spec(spec)
    sys.modules["build_flags"] = mod
    spec.loader.exec_module(mod)
    result = mod.merge_flags("-O2 -Wall", "-march=native")
    if "-O2" not in result:
        print(f"FAIL[packet_loss]: -O2 discarded", file=sys.stderr); return 1
    if "-Wall" not in result:
        print(f"FAIL[packet_loss]: -Wall discarded", file=sys.stderr); return 1
    if "-march=native" not in result:
        print(f"FAIL[packet_loss]: -march=native missing", file=sys.stderr); return 1

    print("PASS")
    return 0
if __name__ == "__main__":
    raise SystemExit(main())
