"""Synthetic cross-compilation target triplet parser — FIXED.

Correctly parses: arch-vendor-kernel-system.
"""

from dataclasses import dataclass


@dataclass
class TargetTriplet:
    arch: str
    vendor: str
    kernel: str
    system: str


def parse_triplet(triplet: str) -> TargetTriplet:
    parts = triplet.split("-")
    if len(parts) != 4:
        raise ValueError(f"Expected 4 parts, got {len(parts)}: {triplet}")
    return TargetTriplet(
        arch=parts[0],
        vendor=parts[1],
        kernel=parts[2],
        system=parts[3],
    )


def format_triplet(t: TargetTriplet) -> str:
    return f"{t.arch}-{t.vendor}-{t.kernel}-{t.system}"
