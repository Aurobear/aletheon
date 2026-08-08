#!/usr/bin/env python3
"""Attestation HMAC"""
import importlib.util, sys, os
def main():
    ws = sys.argv[1] if len(sys.argv) > 1 else "."
    spec = importlib.util.spec_from_file_location("attestation", os.path.join(ws, "attestation.py"))
    mod = importlib.util.module_from_spec(spec)
    sys.modules["attestation"] = mod
    spec.loader.exec_module(mod)
    import hashlib, hmac
    nonce = b"device12345nonce"
    secret = b"presharedsecret"
    key = mod.derive_key(nonce, secret)
    if len(key) != 32:
        print(f"FAIL[endianness]: key length {len(key)} != 32", file=sys.stderr); return 1
    challenge = b"server-challenge"
    expected_key = hashlib.sha256(nonce + secret).digest()
    expected_hmac = hmac.new(expected_key, challenge, hashlib.sha256).digest()
    if not mod.verify_attestation(nonce, secret, challenge, expected_hmac):
        print("FAIL[endianness]: valid attestation should verify", file=sys.stderr); return 1

    print("PASS")
    return 0
if __name__ == "__main__":
    raise SystemExit(main())
