#!/usr/bin/env python3
"""Serial FSM recovery"""
import importlib.util, sys, os
def main():
    ws = sys.argv[1] if len(sys.argv) > 1 else "."
    spec = importlib.util.spec_from_file_location("serial_fsm", os.path.join(ws, "serial_fsm.py"))
    mod = importlib.util.module_from_spec(spec)
    sys.modules["serial_fsm"] = mod
    spec.loader.exec_module(mod)
    port = mod.SerialPort()
    port.state = mod.SerialState.ERROR
    if not port.reset():
        print("FAIL[device_rejection]: cannot recover from ERROR", file=sys.stderr); return 1
    if port.state != mod.SerialState.IDLE:
        print(f"FAIL[device_rejection]: state should be IDLE got {port.state}", file=sys.stderr); return 1

    print("PASS")
    return 0
if __name__ == "__main__":
    raise SystemExit(main())
