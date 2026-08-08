"""Synthetic device attestation HMAC verifier.

BUG: Derives the HMAC key using SHA-1 instead of SHA-256 as required.
The key derivation also builds nonce||secret in the wrong order.
"""
import hashlib
import hmac


def derive_key(device_nonce: bytes, secret: bytes) -> bytes:
    """Derive HMAC key from device nonce and pre-shared secret."""
    # BUG: wrong hash algorithm (SHA-1 instead of SHA-256)
    # BUG: wrong order (secret||nonce instead of nonce||secret)
    return hashlib.sha1(secret + device_nonce).digest()


def verify_attestation(device_nonce: bytes, secret: bytes,
                        challenge: bytes, expected_hmac: bytes) -> bool:
    """Verify device attestation response."""
    key = derive_key(device_nonce, secret)
    computed = hmac.new(key, challenge, hashlib.sha1).digest()
    # BUG: uses SHA-1 for HMAC too
    return hmac.compare_digest(computed, expected_hmac)
