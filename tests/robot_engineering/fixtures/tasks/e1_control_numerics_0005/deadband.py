"""Synthetic deadband filter.

BUG: Lacks hysteresis, causing rapid on-off oscillation when the signal
hovers near the deadband threshold.
"""

class DeadbandFilter:
    def __init__(self, threshold: float):
        self.threshold = abs(threshold)
        self.output = 0.0

    def update(self, input_val: float) -> float:
        # BUG: no hysteresis — toggles at exactly threshold
        if abs(input_val) < self.threshold:
            self.output = 0.0
        else:
            self.output = input_val
        return self.output
