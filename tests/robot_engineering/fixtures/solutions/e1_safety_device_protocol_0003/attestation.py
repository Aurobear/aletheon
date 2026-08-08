"""Synthetic device attestation HMAC verifier — FIXED.

Uses SHA-256 and correct nonce||shared-secret derivation order.
"""
import hashlib
import hmac


def derive_key(device_nonce: bytes, secret: bytes) -> bytes:
    return hashlib.sha256(device_nonce + secret).digest()


def verify_attestation(device_nonce: bytes, secret: bytes,
                        challenge: bytes, expected_hmac: bytes) -> bool:
    key = derive_key(device_nonce, secret)
    computed = hmac.new(key, challenge, hashlib.sha256).digest()
    return hmac.compare_digest(computed, expected_hmac)
