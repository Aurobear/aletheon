"""Synthetic EtherCAT SDO abort code handler.

BUG: Only checks for abort code 0x00000000 (success) but ignores specific
error codes like 0x06010000 (unsupported access).
"""
from enum import IntEnum


class SDOAbortCode(IntEnum):
    SUCCESS = 0x00000000
    UNSUPPORTED_ACCESS = 0x06010000
    OBJECT_DOES_NOT_EXIST = 0x06020000
    DATA_TYPE_MISMATCH = 0x06070000
    GENERAL_ERROR = 0x08000000


class SDOAbortError(Exception):
    def __init__(self, code: int, message: str = ""):
        self.code = code
        self.message = message
        super().__init__(f"SDO Abort 0x{code:08X}: {message}")


def check_sdo_response(abort_code: int) -> bool:
    """Check SDO response. Returns True on success, raises on error."""
    # BUG: only recognizes SUCCESS, all others treated as success
    if abort_code == SDOAbortCode.SUCCESS:
        return True
    # BUG: all other codes silently pass
    return True
