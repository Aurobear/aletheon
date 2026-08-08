"""Synthetic EtherCAT SDO abort code handler — FIXED.

Properly maps specific abort codes to typed exceptions.
"""
from enum import IntEnum


class SDOAbortCode(IntEnum):
    SUCCESS = 0x00000000
    UNSUPPORTED_ACCESS = 0x06010000
    OBJECT_DOES_NOT_EXIST = 0x06020000
    DATA_TYPE_MISMATCH = 0x06070000
    GENERAL_ERROR = 0x08000000


ERROR_MESSAGES = {
    SDOAbortCode.UNSUPPORTED_ACCESS: "Unsupported access to object",
    SDOAbortCode.OBJECT_DOES_NOT_EXIST: "Object does not exist",
    SDOAbortCode.DATA_TYPE_MISMATCH: "Data type mismatch",
    SDOAbortCode.GENERAL_ERROR: "General error",
}


class SDOAbortError(Exception):
    def __init__(self, code: int, message: str = ""):
        self.code = code
        self.message = message
        super().__init__(f"SDO Abort 0x{code:08X}: {message}")


def check_sdo_response(abort_code: int) -> bool:
    if abort_code == SDOAbortCode.SUCCESS:
        return True
    msg = ERROR_MESSAGES.get(abort_code, f"Unknown abort code 0x{abort_code:08X}")
    raise SDOAbortError(abort_code, msg)
