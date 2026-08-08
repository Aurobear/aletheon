#!/usr/bin/env python3
"""SDO abort handler"""
import importlib.util, sys, os
def main():
    ws = sys.argv[1] if len(sys.argv) > 1 else "."
    spec = importlib.util.spec_from_file_location("sdo_abort", os.path.join(ws, "sdo_abort.py"))
    mod = importlib.util.module_from_spec(spec)
    sys.modules["sdo_abort"] = mod
    spec.loader.exec_module(mod)
    if not mod.check_sdo_response(0x00000000):
        print("FAIL[device_rejection]: success code should return True", file=sys.stderr); return 1
    try:
        mod.check_sdo_response(0x06010000)
        print("FAIL[device_rejection]: unsupported access should raise", file=sys.stderr); return 1
    except mod.SDOAbortError:
        pass

    print("PASS")
    return 0
if __name__ == "__main__":
    raise SystemExit(main())
