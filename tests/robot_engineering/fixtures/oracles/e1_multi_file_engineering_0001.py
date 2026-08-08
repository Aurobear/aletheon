#!/usr/bin/env python3
"""Task-specific oracle for e1_multi_file_engineering_0001 — Multi-file interface migration."""
import sys, os, importlib.util
def main():
    ws = sys.argv[1] if len(sys.argv) > 1 else "."
    for fname in ["sensor_hal.py", "controller.py", "diagnostics.py"]:
        with open(os.path.join(ws, fname)) as f:
            src = f.read()
        if "read_sensor" in src and fname != "oracle.py":
            print(f"FAIL[device_rejection]: {fname} still has read_sensor", file=sys.stderr); return 1
        if fname == "sensor_hal.py" and "def read_channel" not in src:
            print("FAIL[device_rejection]: sensor_hal.py missing read_channel", file=sys.stderr); return 1
        if fname != "sensor_hal.py" and "read_channel" not in src:
            print(f"FAIL[device_rejection]: {fname} does not use read_channel", file=sys.stderr); return 1
    spec = importlib.util.spec_from_file_location("hal", os.path.join(ws, "sensor_hal.py"))
    hal_mod = importlib.util.module_from_spec(spec)
    sys.modules["hal"] = hal_mod
    spec.loader.exec_module(hal_mod)
    hal = hal_mod.SensorHAL()
    hal.set_channel(1, 3.14)
    if hal.read_channel(1) != 3.14:
        print("FAIL[device_rejection]: read_channel incorrect", file=sys.stderr); return 1
    print("PASS")
    return 0
if __name__ == "__main__":
    raise SystemExit(main())
