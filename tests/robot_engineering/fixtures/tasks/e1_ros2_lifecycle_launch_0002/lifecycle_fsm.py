"""Synthetic ROS 2 lifecycle state machine.

BUG: The state machine allows transitioning directly from unconfigured to active,
skipping the required inactive state per the ROS 2 managed node specification.
"""
from enum import Enum


class LifecycleState(Enum):
    UNCONFIGURED = 0
    INACTIVE = 1
    ACTIVE = 2
    FINALIZED = 3
    ERROR = 4


VALID_TRANSITIONS = {
    LifecycleState.UNCONFIGURED: {LifecycleState.INACTIVE, LifecycleState.ACTIVE, LifecycleState.FINALIZED},
    LifecycleState.INACTIVE: {LifecycleState.ACTIVE, LifecycleState.UNCONFIGURED, LifecycleState.FINALIZED},
    LifecycleState.ACTIVE: {LifecycleState.INACTIVE, LifecycleState.FINALIZED},
    LifecycleState.FINALIZED: set(),
    LifecycleState.ERROR: {LifecycleState.UNCONFIGURED},
}


class LifecycleNode:
    def __init__(self):
        self.state = LifecycleState.UNCONFIGURED

    def transition_to(self, target: LifecycleState) -> bool:
        if target in VALID_TRANSITIONS.get(self.state, set()):
            self.state = target
            return True
        return False

    def configure(self) -> bool:
        return self.transition_to(LifecycleState.INACTIVE)

    def activate(self) -> bool:
        return self.transition_to(LifecycleState.ACTIVE)

    def deactivate(self) -> bool:
        return self.transition_to(LifecycleState.INACTIVE)

    def cleanup(self) -> bool:
        return self.transition_to(LifecycleState.UNCONFIGURED)

    def shutdown(self) -> bool:
        return self.transition_to(LifecycleState.FINALIZED)
