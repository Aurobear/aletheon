"""Sensor Hardware Abstraction Layer — FIXED.

Renamed the old single-channel read to read_channel with explicit channel_id.
"""

class SensorHAL:
    def __init__(self, num_channels: int = 4):
        self.num_channels = num_channels
        self._values = [0.0] * num_channels

    def read_channel(self, channel_id: int) -> float:
        if 0 <= channel_id < self.num_channels:
            return self._values[channel_id]
        raise IndexError(f"Channel {channel_id} out of range")

    def set_channel(self, channel_id: int, value: float):
        if 0 <= channel_id < self.num_channels:
            self._values[channel_id] = value
