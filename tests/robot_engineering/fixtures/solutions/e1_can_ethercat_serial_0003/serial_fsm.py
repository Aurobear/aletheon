"""Synthetic serial port state machine — FIXED.

ERROR state has a recovery transition back to IDLE.
"""
from enum import Enum


class SerialState(Enum):
    IDLE = 0
    OPENING = 1
    CONNECTED = 2
    CLOSING = 3
    ERROR = 4


TRANSITIONS = {
    SerialState.IDLE: {SerialState.OPENING},
    SerialState.OPENING: {SerialState.CONNECTED, SerialState.ERROR},
    SerialState.CONNECTED: {SerialState.CLOSING, SerialState.ERROR},
    SerialState.CLOSING: {SerialState.IDLE, SerialState.ERROR},
    SerialState.ERROR: {SerialState.IDLE},
}


class SerialPort:
    def __init__(self):
        self.state = SerialState.IDLE

    def transition(self, target: SerialState) -> bool:
        if target in TRANSITIONS.get(self.state, set()):
            self.state = target
            return True
        return False

    def open(self) -> bool:
        return self.transition(SerialState.OPENING)

    def connect(self) -> bool:
        return self.transition(SerialState.CONNECTED)

    def close(self) -> bool:
        return self.transition(SerialState.CLOSING)

    def reset(self) -> bool:
        return self.transition(SerialState.IDLE)
