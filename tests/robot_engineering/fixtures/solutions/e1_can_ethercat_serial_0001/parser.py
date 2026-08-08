"""Synthetic EtherCAT frame parser — FIXED.

Decodes the two-byte length big-endian regardless of host byte order.
"""


def decode_length(wire_hi: int, wire_lo: int) -> int:
    return (wire_hi << 8) | wire_lo


def parse_frame(data: bytes) -> dict:
    if len(data) < 2:
        raise ValueError("frame too short")
    length = decode_length(data[0], data[1])
    return {"length": length, "payload": data[2:2 + length]}
