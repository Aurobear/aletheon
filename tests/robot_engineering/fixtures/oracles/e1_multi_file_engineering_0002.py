#!/usr/bin/env python3
"""Task-specific oracle for e1_multi_file_engineering_0002 — Test coverage completion."""
import sys, os, subprocess
def main():
    ws = sys.argv[1] if len(sys.argv) > 1 else "."
    test_path = os.path.join(ws, "test_mathlib.py")
    with open(test_path) as f:
        src = f.read()
    errors = []
    if "def test_lerp" not in src:
        errors.append("missing test_lerp")
    if "def test_normalize_angle" not in src:
        errors.append("missing test_normalize_angle")
    if errors:
        for e in errors:
            print(f"FAIL[packet_loss]: {e}", file=sys.stderr)
        return 1
    result = subprocess.run([sys.executable, test_path], capture_output=True, text=True, cwd=ws)
    if result.returncode != 0:
        print(f"FAIL[packet_loss]: tests did not pass", file=sys.stderr); return 1
    print("PASS")
    return 0
if __name__ == "__main__":
    raise SystemExit(main())
