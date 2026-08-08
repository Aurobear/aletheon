"""Robot controller that reads sensor data — FIXED.

Uses new read_channel() API.
"""
import sensor_hal


class Controller:
    def __init__(self):
        self.hal = sensor_hal.SensorHAL(num_channels=4)

    def get_sensor_value(self) -> float:
        return self.hal.read_channel(0)

    def update(self):
        val = self.get_sensor_value()
        return val * 2.0
