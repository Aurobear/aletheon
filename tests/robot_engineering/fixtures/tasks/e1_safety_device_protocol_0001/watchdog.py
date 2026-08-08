"""Synthetic safety watchdog stub.

The watchdog only resets its timer on a full-throttle heartbeat, so a partially
loaded controller stalls the reset and trips a false safe-stop. The fix resets
the timer on every heartbeat and only arms the stop when the heartbeat age
exceeds the configured ceiling.
"""


class Watchdog:
    def __init__(self, ceiling_ms: int = 500):
        self.ceiling_ms = ceiling_ms
        self.last_heartbeat_ms = 0
        self.armed = False

    def heartbeat(self, now_ms: int) -> None:
        # BUG: only updates on a full-throttle tick; partial loads stall the
        # reset and falsely trip the stop.
        if now_ms % 200 == 0:
            self.last_heartbeat_ms = now_ms

    def evaluate(self, now_ms: int) -> bool:
        age = now_ms - self.last_heartbeat_ms
        self.armed = age > self.ceiling_ms
        return self.armed
