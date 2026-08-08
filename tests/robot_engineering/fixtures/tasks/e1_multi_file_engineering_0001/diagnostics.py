"""Diagnostics module that monitors sensor health.

BUG: Uses deprecated read_sensor() API.
"""
import sensor_hal


class Diagnostics:
    def __init__(self):
        self.hal = sensor_hal.SensorHAL(num_channels=4)

    def check_health(self) -> dict:
        # BUG: uses deprecated read_sensor()
        raw = self.hal.read_sensor()
        return {
            "channel_0": raw,
            "status": "ok" if raw > 0.1 else "low_signal",
        }
