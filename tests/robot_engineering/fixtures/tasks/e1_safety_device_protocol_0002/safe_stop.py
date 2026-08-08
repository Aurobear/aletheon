"""Synthetic safe-stop sequencer for robot joint.

BUG: Executes the stop sequence in the wrong order: it releases the brake
before decelerating the motor to zero velocity, causing the joint to fall
under gravity. Should decelerate first, then engage brake.
"""
from typing import List, Tuple
from enum import Enum


class StopPhase(Enum):
    IDLE = 0
    RELEASE_BRAKE = 1
    DECELERATE = 2
    ENGAGE_BRAKE = 3
    COMPLETE = 4


class SafeStopSequencer:
    def __init__(self):
        self.phase = StopPhase.IDLE
        self.velocity = 0.0
        self.brake_engaged = True

    def initiate_stop(self, current_velocity: float):
        self.velocity = current_velocity
        self.phase = StopPhase.RELEASE_BRAKE
        # BUG: releases brake before decelerating
        self.brake_engaged = False

    def step(self) -> StopPhase:
        """Execute one step of the stop sequence."""
        if self.phase == StopPhase.RELEASE_BRAKE:
            # BUG: brake already released, joint falls under gravity
            self.phase = StopPhase.DECELERATE
        elif self.phase == StopPhase.DECELERATE:
            # Decelerate at 10 units/s^2
            if self.velocity > 0:
                self.velocity = max(0, self.velocity - 10.0 * 0.01)
            self.phase = StopPhase.ENGAGE_BRAKE
        elif self.phase == StopPhase.ENGAGE_BRAKE:
            # BUG: engages brake before velocity reaches zero
            self.brake_engaged = True
            self.phase = StopPhase.COMPLETE
        return self.phase

    def is_safe(self) -> bool:
        return self.velocity == 0.0 and self.brake_engaged
