#!/usr/bin/env python3
"""CAN bus-off recovery"""
import importlib.util, sys, os
def main():
    ws = sys.argv[1] if len(sys.argv) > 1 else "."
    spec = importlib.util.spec_from_file_location("can_busoff", os.path.join(ws, "can_busoff.py"))
    mod = importlib.util.module_from_spec(spec)
    sys.modules["can_busoff"] = mod
    spec.loader.exec_module(mod)
    r = mod.CANBusOffRecovery()
    r.enter_bus_off()
    for _ in range(5):
        r.attempt_recovery()
    if r.recovery_count < 3:
        print(f"FAIL[watchdog]: recovery_count={r.recovery_count} < 3", file=sys.stderr); return 1

    print("PASS")
    return 0
if __name__ == "__main__":
    raise SystemExit(main())
