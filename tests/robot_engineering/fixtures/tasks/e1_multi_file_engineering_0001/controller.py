"""Robot controller that reads sensor data.

BUG: Uses deprecated read_sensor() API.
"""
import sensor_hal


class Controller:
    def __init__(self):
        self.hal = sensor_hal.SensorHAL(num_channels=4)

    def get_sensor_value(self) -> float:
        # BUG: uses deprecated read_sensor()
        return self.hal.read_sensor()

    def update(self):
        val = self.get_sensor_value()
        return val * 2.0
