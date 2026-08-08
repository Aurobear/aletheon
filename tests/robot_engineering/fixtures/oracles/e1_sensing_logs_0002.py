#!/usr/bin/env python3
"""Drop detection"""
import importlib.util, sys, os
def main():
    ws = sys.argv[1] if len(sys.argv) > 1 else "."
    spec = importlib.util.spec_from_file_location("drop_detect", os.path.join(ws, "drop_detect.py"))
    mod = importlib.util.module_from_spec(spec)
    sys.modules["drop_detect"] = mod
    spec.loader.exec_module(mod)
    det = mod.DropDetector()
    total_drops = 0
    for seq in [1, 2, 3, 7, 8, 10]:
        total_drops += det.process_packet(seq)
    if total_drops < 4:
        print(f"FAIL[packet_loss]: expected >=4 drops, got {total_drops}", file=sys.stderr); return 1

    print("PASS")
    return 0
if __name__ == "__main__":
    raise SystemExit(main())
