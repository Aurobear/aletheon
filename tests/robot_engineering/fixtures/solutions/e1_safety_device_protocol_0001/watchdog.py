"""Synthetic safety watchdog stub — FIXED.

Resets the timer on every heartbeat and only arms the stop when heartbeat age
exceeds the configured ceiling.
"""


class Watchdog:
    def __init__(self, ceiling_ms: int = 500):
        self.ceiling_ms = ceiling_ms
        self.last_heartbeat_ms = 0
        self.armed = False

    def heartbeat(self, now_ms: int) -> None:
        self.last_heartbeat_ms = now_ms

    def evaluate(self, now_ms: int) -> bool:
        age = now_ms - self.last_heartbeat_ms
        self.armed = age > self.ceiling_ms
        return self.armed
