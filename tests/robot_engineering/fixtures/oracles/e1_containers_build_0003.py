#!/usr/bin/env python3
"""Task-specific oracle for e1_containers_build_0003 — Cross-compilation triplet."""
import importlib.util, sys, os
def main():
    ws = sys.argv[1] if len(sys.argv) > 1 else "."
    spec = importlib.util.spec_from_file_location("ct", os.path.join(ws, "cross_triplet.py"))
    mod = importlib.util.module_from_spec(spec)
    sys.modules["ct"] = mod
    spec.loader.exec_module(mod)
    t = mod.parse_triplet("x86_64-pc-linux-gnu")
    if t.arch != "x86_64":
        print(f"FAIL[endianness]: arch expected x86_64 got {t.arch}", file=sys.stderr); return 1
    if t.vendor != "pc":
        print(f"FAIL[endianness]: vendor expected pc got {t.vendor}", file=sys.stderr); return 1
    if t.kernel != "linux":
        print(f"FAIL[endianness]: kernel expected linux got {t.kernel}", file=sys.stderr); return 1
    if t.system != "gnu":
        print(f"FAIL[endianness]: system expected gnu got {t.system}", file=sys.stderr); return 1
    print("PASS")
    return 0
if __name__ == "__main__":
    raise SystemExit(main())
