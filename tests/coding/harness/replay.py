#!/usr/bin/env python3
"""Fail-closed integrity and evidence replay for coding receipts."""
import hashlib, json, pathlib, sys

def digest(data): return "sha256:"+hashlib.sha256(data).hexdigest()
def verify(path):
    receipt=json.loads(pathlib.Path(path).read_text()); expected=receipt.pop("integrity_sha256","")
    actual=digest(json.dumps(receipt,sort_keys=True,separators=(",", ":")).encode())
    if expected != actual: return False, "receipt integrity mismatch"
    op=receipt.get("operation_id",""); evidence=receipt.get("evidence",[])
    if not op or not evidence or any(e.get("operation_id")!=op for e in evidence): return False,"operation evidence mismatch"
    if not receipt.get("workspace_diff","").strip(): return False,"workspace diff missing"
    if not any(e.get("kind")=="acceptance_command" and e.get("exit_code")==0 for e in evidence): return False,"successful command evidence missing"
    acceptance=receipt.get("acceptance",[])
    if not acceptance or any(x.get("exit_code")!=0 or x.get("timed_out") for x in acceptance): return False,"acceptance failed"
    if not receipt.get("verification",{}).get("passed") or receipt.get("terminal_status")!="verified": return False,"false success"
    return True,"verified"
if __name__=="__main__":
    ok,msg=verify(sys.argv[1]); print(msg); raise SystemExit(0 if ok else 1)
