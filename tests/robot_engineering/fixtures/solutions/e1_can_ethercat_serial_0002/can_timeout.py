"""Synthetic CAN bus frame receive timeout — FIXED.

Uses wall-clock time for timeout detection.
"""
import time


class CANReceiver:
    def __init__(self, timeout_ms: int = 100):
        self.timeout_ms = timeout_ms
        self._start_time = 0.0
        self._polls = 0

    def start_receive(self):
        self._start_time = time.monotonic()
        self._polls = 0

    def poll(self) -> bool:
        self._polls += 1
        elapsed = (time.monotonic() - self._start_time) * 1000
        if elapsed > self.timeout_ms:
            return False
        return self._polls % 10 == 0

    def elapsed_ms(self) -> int:
        return int((time.monotonic() - self._start_time) * 1000)
