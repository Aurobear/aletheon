"""Synthetic CMake build flags merger — FIXED.

Appends injection flags to existing flags instead of overwriting.
"""

class BuildFlags:
    def __init__(self, existing_flags: str = ""):
        self.flags = existing_flags

    def inject(self, new_flags: str) -> str:
        if self.flags:
            self.flags = self.flags + " " + new_flags
        else:
            self.flags = new_flags
        return self.flags

    def get_flags(self) -> str:
        return self.flags


def merge_flags(base: str, injection: str) -> str:
    bf = BuildFlags(base)
    return bf.inject(injection)
