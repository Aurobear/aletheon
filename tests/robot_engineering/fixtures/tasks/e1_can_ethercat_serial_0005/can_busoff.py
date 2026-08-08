"""Synthetic CAN bus-off recovery handler.

BUG: Increments recovery counter only on successful transmission, not on
the recovery attempt itself. Per CAN spec, counter increments on each attempt.
"""

class CANBusOffRecovery:
    MAX_RECOVERY_ATTEMPTS = 5

    def __init__(self):
        self.recovery_count = 0
        self.bus_off = False
        self.transmit_ok = False

    def enter_bus_off(self):
        self.bus_off = True
        self.transmit_ok = False

    def attempt_recovery(self) -> bool:
        """Attempt to recover from bus-off. Returns True if recovered."""
        if not self.bus_off:
            return True
        # Simulate recovery attempt (fails first 3 times)
        self.transmit_ok = self.recovery_count >= 3
        # BUG: only increments on successful transmit
        if self.transmit_ok:
            self.recovery_count += 1
            self.bus_off = False
            return True
        return False

    def should_escalate(self) -> bool:
        return self.recovery_count >= self.MAX_RECOVERY_ATTEMPTS
