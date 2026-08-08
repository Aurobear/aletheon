#!/usr/bin/env python3
"""Task-specific oracle for e1_containers_build_0001 — Docker multistage cache ordering."""
import sys, os
def main():
    ws = sys.argv[1] if len(sys.argv) > 1 else "."
    with open(os.path.join(ws, "Dockerfile.multistage")) as f:
        content = f.read()
    lines = content.split("\n")
    copy_req_idx = None
    copy_source_idx = None
    pip_install_idx = None
    for i, line in enumerate(lines):
        if "COPY" in line and "requirements" in line and "from=builder" not in line.lower():
            copy_req_idx = i
        if "COPY . " in line and "from=builder" not in line.lower():
            copy_source_idx = i
        if "pip install" in line and "requirements" in line:
            pip_install_idx = i
    if copy_req_idx is None:
        print("FAIL[latency]: missing COPY requirements.txt", file=sys.stderr); return 1
    if copy_source_idx is not None and pip_install_idx is not None and copy_source_idx < pip_install_idx:
        print("FAIL[latency]: source copied before pip install", file=sys.stderr); return 1
    print("PASS")
    return 0
if __name__ == "__main__":
    raise SystemExit(main())
