"""Synthetic deadband filter — FIXED.

Uses symmetric hysteresis with separate rising and falling thresholds.
"""

class DeadbandFilter:
    def __init__(self, threshold: float, hysteresis: float = None):
        self.threshold = abs(threshold)
        self.hysteresis = abs(hysteresis) if hysteresis is not None else self.threshold * 0.2
        self.rising_threshold = self.threshold + self.hysteresis
        self.falling_threshold = self.threshold - self.hysteresis
        self.output = 0.0
        self.active = False

    def update(self, input_val: float) -> float:
        if self.active:
            if abs(input_val) < self.falling_threshold:
                self.active = False
                self.output = 0.0
            else:
                self.output = input_val
        else:
            if abs(input_val) > self.rising_threshold:
                self.active = True
                self.output = input_val
            else:
                self.output = 0.0
        return self.output
