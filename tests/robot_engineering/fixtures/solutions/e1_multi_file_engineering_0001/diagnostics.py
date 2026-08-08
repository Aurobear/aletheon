"""Diagnostics module — FIXED.

Uses new read_channel() API.
"""
import sensor_hal


class Diagnostics:
    def __init__(self):
        self.hal = sensor_hal.SensorHAL(num_channels=4)

    def check_health(self) -> dict:
        raw = self.hal.read_channel(0)
        return {
            "channel_0": raw,
            "status": "ok" if raw > 0.1 else "low_signal",
        }
