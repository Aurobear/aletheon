#!/usr/bin/env python3
"""Task-specific oracle for e1_ros2_lifecycle_launch_0005 — Node discovery backoff."""
import importlib.util, sys, os, time
def main():
    ws = sys.argv[1] if len(sys.argv) > 1 else "."
    spec = importlib.util.spec_from_file_location("nd", os.path.join(ws, "node_discovery.py"))
    mod = importlib.util.module_from_spec(spec)
    sys.modules["nd"] = mod
    spec.loader.exec_module(mod)
    nd = mod.NodeDiscovery()
    # Run 4 discoveries and measure delays
    delays = []
    for _ in range(4):
        t0 = time.monotonic()
        nd.discover_peers()
        dt = (time.monotonic() - t0) * 1000
        delays.append(round(dt, 1))
    nd.reset()
    t0 = time.monotonic()
    nd.discover_peers()
    first_delay = round((time.monotonic() - t0) * 1000, 1)
    # Buggy: fixed 100ms delay. All delays cluster near 100ms.
    # Fixed: backoff + jitter. Delays should vary or increase.
    delay_spread = max(delays) - min(delays)
    near_100ms = abs(first_delay - 100) < 25
    if delay_spread < 12 and near_100ms:
        print("FAIL[clock_drift]: all delays identical (~100ms), no backoff/jitter", file=sys.stderr)
        return 1
    print("PASS")
    return 0
if __name__ == "__main__":
    raise SystemExit(main())
