#!/usr/bin/env python3
"""Bag index corruption detection"""
import importlib.util, sys, os
def main():
    ws = sys.argv[1] if len(sys.argv) > 1 else "."
    spec = importlib.util.spec_from_file_location("bag_index", os.path.join(ws, "bag_index.py"))
    mod = importlib.util.module_from_spec(spec)
    sys.modules["bag_index"] = mod
    spec.loader.exec_module(mod)

    parser = mod.BagIndexParser()
    parser.add_raw_chunk(0, 0, 1000, 10)
    parser.add_raw_chunk(1, -1, 1000, 5)
    parser.add_raw_chunk(2, 2000, 200_000_000, 20)
    parser.add_raw_chunk(3, 3000, 500, 8)
    valid = parser.get_valid_entries()
    ids = [e.chunk_id for e in valid]
    if 1 in ids or 2 in ids:
        print("FAIL[packet_loss]: corrupted chunks should be rejected", file=sys.stderr); return 1
    if len(valid) != 2:
        print(f"FAIL[packet_loss]: expected 2 valid, got {len(valid)}", file=sys.stderr); return 1

    print("PASS")
    return 0
if __name__ == "__main__":
    raise SystemExit(main())
