"""Synthetic cross-compilation target triplet parser.

BUG: Swaps the vendor and kernel fields when parsing,
producing the wrong target tuple. Correct mapping: arch-vendor-kernel-system.
"""

from dataclasses import dataclass


@dataclass
class TargetTriplet:
    arch: str
    vendor: str
    kernel: str
    system: str


def parse_triplet(triplet: str) -> TargetTriplet:
    """Parse a GCC target triplet: arch-vendor-kernel-system."""
    parts = triplet.split("-")
    if len(parts) != 4:
        raise ValueError(f"Expected 4 parts, got {len(parts)}: {triplet}")
    # BUG: vendor and kernel are swapped
    return TargetTriplet(
        arch=parts[0],
        vendor=parts[2],  # should be parts[1]
        kernel=parts[1],  # should be parts[2]
        system=parts[3],
    )


def format_triplet(t: TargetTriplet) -> str:
    return f"{t.arch}-{t.vendor}-{t.kernel}-{t.system}"
