"""Synthetic safe-stop sequencer — FIXED.

Decelerates the motor to zero velocity before engaging the brake.
"""
from typing import List, Tuple
from enum import Enum


class StopPhase(Enum):
    IDLE = 0
    DECELERATE = 1
    ENGAGE_BRAKE = 2
    COMPLETE = 3


class SafeStopSequencer:
    def __init__(self):
        self.phase = StopPhase.IDLE
        self.velocity = 0.0
        self.brake_engaged = True

    def initiate_stop(self, current_velocity: float):
        self.velocity = current_velocity
        self.phase = StopPhase.DECELERATE

    def step(self) -> StopPhase:
        if self.phase == StopPhase.DECELERATE:
            if self.velocity > 0:
                self.velocity = max(0, self.velocity - 10.0 * 0.01)
            if self.velocity == 0.0:
                self.phase = StopPhase.ENGAGE_BRAKE
        elif self.phase == StopPhase.ENGAGE_BRAKE:
            self.brake_engaged = True
            self.phase = StopPhase.COMPLETE
        return self.phase

    def is_safe(self) -> bool:
        return self.velocity == 0.0 and self.brake_engaged
