"""Synthetic CAN bus frame receive timeout.

BUG: Uses a simple counter-based timeout instead of wall-clock time.
A slow clock or fast frames causes premature timeout or missed frames.
"""
import time


class CANReceiver:
    def __init__(self, timeout_ms: int = 100):
        self.timeout_ms = timeout_ms
        self.ticks = 0

    def start_receive(self):
        self.ticks = 0

    def poll(self) -> bool:
        """Check if a frame is ready. Returns True if data available."""
        self.ticks += 1
        # BUG: counter-based timeout ignores real time
        if self.ticks > self.timeout_ms:
            return False  # timeout
        return self.ticks % 10 == 0  # mock: frame every 10 polls

    def elapsed_ms(self) -> int:
        return self.ticks  # BUG: ticks != milliseconds
