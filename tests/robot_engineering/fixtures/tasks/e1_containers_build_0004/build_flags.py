"""Synthetic CMake build flags merger.

BUG: Discards existing CMAKE_CXX_FLAGS when applying injection flags,
instead of appending.
"""

class BuildFlags:
    def __init__(self, existing_flags: str = ""):
        self.flags = existing_flags

    def inject(self, new_flags: str) -> str:
        # BUG: overwrites instead of appending
        self.flags = new_flags
        return self.flags

    def get_flags(self) -> str:
        return self.flags


def merge_flags(base: str, injection: str) -> str:
    bf = BuildFlags(base)
    return bf.inject(injection)
