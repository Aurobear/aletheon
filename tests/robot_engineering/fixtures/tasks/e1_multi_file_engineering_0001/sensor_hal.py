"""Sensor Hardware Abstraction Layer.

BUG: Uses old function name read_sensor() which needs to be migrated to
read_channel(channel_id).
"""

class SensorHAL:
    def __init__(self, num_channels: int = 4):
        self.num_channels = num_channels
        self._values = [0.0] * num_channels

    def read_sensor(self) -> float:
        """Read the default sensor (channel 0). DEPRECATED."""
        return self._values[0]

    def set_channel(self, channel_id: int, value: float):
        if 0 <= channel_id < self.num_channels:
            self._values[channel_id] = value
