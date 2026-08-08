#!/usr/bin/env python3
"""Task-specific oracle for e1_containers_build_0002 — Colcon build order diamond deps."""
import importlib.util, sys, os
def main():
    ws = sys.argv[1] if len(sys.argv) > 1 else "."
    spec = importlib.util.spec_from_file_location("cbo", os.path.join(ws, "colcon_build_order.py"))
    mod = importlib.util.module_from_spec(spec)
    sys.modules["cbo"] = mod
    spec.loader.exec_module(mod)
    g = mod.PackageGraph()
    g.add_package("D")
    g.add_package("B", ["D"])
    g.add_package("C", ["D"])
    g.add_package("A", ["B", "C"])
    order = g.build_order()
    d_idx = order.index("D")
    for pkg in ["A", "B", "C"]:
        if d_idx > order.index(pkg):
            print(f"FAIL[reordering]: D must come before {pkg}, got order={order}", file=sys.stderr); return 1
    print("PASS")
    return 0
if __name__ == "__main__":
    raise SystemExit(main())
