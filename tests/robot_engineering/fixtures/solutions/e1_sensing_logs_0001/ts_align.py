"""Synthetic multi-sensor timestamp alignment — FIXED.

Converts sample index to time using sensor timebase.
"""
from typing import List, Tuple


def align_to_master(sensor_samples: List[Tuple[int, float]],
                     sensor_hz: float,
                     master_start_s: float) -> List[Tuple[float, float]]:
    result = []
    for idx, val in sensor_samples:
        ts = master_start_s + idx / sensor_hz
        result.append((ts, val))
    return result
