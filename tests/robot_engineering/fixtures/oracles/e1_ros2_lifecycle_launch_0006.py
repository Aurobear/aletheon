#!/usr/bin/env python3
"""Topic remapping namespaces"""
import importlib.util, sys, os
def main():
    ws = sys.argv[1] if len(sys.argv) > 1 else "."
    spec = importlib.util.spec_from_file_location("topic_remap", os.path.join(ws, "topic_remap.py"))
    mod = importlib.util.module_from_spec(spec)
    sys.modules["topic_remap"] = mod
    spec.loader.exec_module(mod)
    rm = mod.TopicRemapper("/robot/base/")
    result = rm.remap("scan")
    if "//" in result:
        print("FAIL[device_rejection]: double slash", file=sys.stderr); return 1
    if result != "/robot/base/scan":
        print(f"FAIL[device_rejection]: expected /robot/base/scan got {result}", file=sys.stderr); return 1
    if rm.remap("/global/scan") != "/global/scan":
        print("FAIL[device_rejection]: absolute topic mangled", file=sys.stderr); return 1

    print("PASS")
    return 0
if __name__ == "__main__":
    raise SystemExit(main())
