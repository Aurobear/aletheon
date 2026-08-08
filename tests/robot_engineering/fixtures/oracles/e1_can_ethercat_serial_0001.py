#!/usr/bin/env python3
"""Task-specific oracle for e1_can_ethercat_serial_0001 — EtherCAT endianness."""
import importlib.util, sys, os
def main():
    ws = sys.argv[1] if len(sys.argv) > 1 else "."
    spec = importlib.util.spec_from_file_location("parser", os.path.join(ws, "parser.py"))
    mod = importlib.util.module_from_spec(spec)
    sys.modules["parser"] = mod
    spec.loader.exec_module(mod)
    wire = bytes.fromhex("0201") + b"\x00" * 8
    parsed = mod.parse_frame(wire)
    if parsed["length"] != 513:
        print(f"FAIL[endianness]: decoded {parsed['length']}, expected 513", file=sys.stderr); return 1
    print("PASS")
    return 0
if __name__ == "__main__":
    raise SystemExit(main())
