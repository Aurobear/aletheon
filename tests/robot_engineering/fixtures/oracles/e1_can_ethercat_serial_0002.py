#!/usr/bin/env python3
"""Task-specific oracle for e1_can_ethercat_serial_0002 — CAN frame timeout."""
import importlib.util, sys, os, time
def main():
    ws = sys.argv[1] if len(sys.argv) > 1 else "."
    spec = importlib.util.spec_from_file_location("can", os.path.join(ws, "can_timeout.py"))
    mod = importlib.util.module_from_spec(spec)
    sys.modules["can"] = mod
    spec.loader.exec_module(mod)
    # Test: after exactly 100 polls, elapsed_ms() in a correct implementation
    # should report wall-clock time (~0-10ms), not 100 ticks.
    # In the buggy version, elapsed_ms() returns the tick counter (100).
    rx = mod.CANReceiver(timeout_ms=5000)
    rx.start_receive()
    for _ in range(100):
        rx.poll()
    reported = rx.elapsed_ms()
    # Buggy: reported == 100 (tick count). Fixed: reported < 50 (wall time in ms).
    if reported > 90:
        print(f"FAIL[clock_drift]: elapsed_ms()={reported} looks like tick counter, not wall time", file=sys.stderr)
        return 1
    print("PASS")
    return 0
if __name__ == "__main__":
    raise SystemExit(main())
