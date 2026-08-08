#!/usr/bin/env python3
"""Task-specific oracle for e1_ros2_lifecycle_launch_0003 — Parameter resolver priority."""
import importlib.util, sys, os
def main():
    ws = sys.argv[1] if len(sys.argv) > 1 else "."
    spec = importlib.util.spec_from_file_location("pr", os.path.join(ws, "param_resolver.py"))
    mod = importlib.util.module_from_spec(spec)
    sys.modules["pr"] = mod
    spec.loader.exec_module(mod)
    r = mod.ParamResolver({"topic": "/default", "rate": 10})
    r.add_launch_override("topic", "/launch_val")
    r.add_cli_override("topic", "/cli_val")
    val = r.resolve("topic")
    if val != "/cli_val":
        print(f"FAIL[reordering]: expected /cli_val got {val}", file=sys.stderr); return 1
    print("PASS")
    return 0
if __name__ == "__main__":
    raise SystemExit(main())
