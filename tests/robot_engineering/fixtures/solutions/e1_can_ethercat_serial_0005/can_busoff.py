"""Synthetic CAN bus-off recovery handler — FIXED.

Increments recovery counter on each recovery attempt per CAN specification.
"""

class CANBusOffRecovery:
    MAX_RECOVERY_ATTEMPTS = 5

    def __init__(self):
        self.recovery_count = 0
        self.bus_off = False

    def enter_bus_off(self):
        self.bus_off = True

    def attempt_recovery(self) -> bool:
        if not self.bus_off:
            return True
        self.recovery_count += 1
        if self.recovery_count >= 3:
            self.bus_off = False
            return True
        return False

    def should_escalate(self) -> bool:
        return self.recovery_count >= self.MAX_RECOVERY_ATTEMPTS
