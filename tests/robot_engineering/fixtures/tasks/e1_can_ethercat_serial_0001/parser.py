"""Synthetic EtherCAT frame parser.

The data length is serialized big-endian on the wire but decoded as native
little-endian. A target little-endian controller reads 0x0201 as 513 instead of
258. The fix is to decode the two-byte length big-endian regardless of host
byte order.
"""


def decode_length(wire_hi: int, wire_lo: int) -> int:
    # BUG: native-endian interpretation mis-orders the two wire bytes.
    return (wire_lo << 8) | wire_hi


def parse_frame(data: bytes) -> dict:
    if len(data) < 2:
        raise ValueError("frame too short")
    length = decode_length(data[0], data[1])
    return {"length": length, "payload": data[2:2 + length]}
