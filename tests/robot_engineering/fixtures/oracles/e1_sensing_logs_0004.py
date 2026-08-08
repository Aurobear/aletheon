#!/usr/bin/env python3
"""Log diagnosis severity"""
import importlib.util, sys, os
def main():
    ws = sys.argv[1] if len(sys.argv) > 1 else "."
    spec = importlib.util.spec_from_file_location("log_diag", os.path.join(ws, "log_diag.py"))
    mod = importlib.util.module_from_spec(spec)
    sys.modules["log_diag"] = mod
    spec.loader.exec_module(mod)
    logs = [
        "DEBUG Starting sensor poll",
        "INFO Sensor data received",
        "WARN Temperature above nominal",
        "ERROR Motor controller fault",
        "FATAL Emergency stop triggered",
        "INFO Normal operation",
    ]
    anomalies = mod.detect_anomalies(logs, min_severity="WARN")
    sevs = [a["severity"] for a in anomalies]
    if "INFO" in sevs:
        print(f"FAIL[device_rejection]: INFO in anomalies", file=sys.stderr); return 1
    if "ERROR" not in sevs:
        print(f"FAIL[device_rejection]: ERROR missing", file=sys.stderr); return 1
    if "FATAL" not in sevs:
        print(f"FAIL[device_rejection]: FATAL missing", file=sys.stderr); return 1

    print("PASS")
    return 0
if __name__ == "__main__":
    raise SystemExit(main())
