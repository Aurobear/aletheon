"""Synthetic multi-sensor timestamp alignment.

BUG: Uses the sensor's internal sample index as a timestamp offset without
converting to the common master clock timebase. This causes drift when
sensor clocks run at different rates.
"""
from typing import List, Tuple


def align_to_master(sensor_samples: List[Tuple[int, float]],
                     sensor_hz: float,
                     master_start_s: float) -> List[Tuple[float, float]]:
    """Align sensor samples to master clock.

    Args:
        sensor_samples: list of (sample_index, value)
        sensor_hz: sensor sample rate in Hz
        master_start_s: master clock timestamp when sensor stream started

    Returns:
        list of (timestamp_master_s, value)
    """
    result = []
    for idx, val in sensor_samples:
        # BUG: uses raw sample index without timebase conversion
        ts = master_start_s + idx  # WRONG: idx is not seconds
        result.append((ts, val))
    return result
