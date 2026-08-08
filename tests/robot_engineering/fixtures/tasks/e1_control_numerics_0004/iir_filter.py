"""Synthetic IIR (Infinite Impulse Response) filter.

BUG: Applies feedforward coefficients (b) in reverse order, causing frequency
response distortion. b[0] should apply to the current input sample.
"""
from typing import List


class IIRFilter:
    def __init__(self, b: List[float], a: List[float]):
        """b = feedforward coeffs, a = feedback coeffs (a[0] is typically 1.0)."""
        self.b = list(b)
        self.a = list(a)
        self.x_buf = [0.0] * len(b)
        self.y_buf = [0.0] * (len(a) - 1)

    def process(self, x: float) -> float:
        # Shift input buffer
        self.x_buf = [x] + self.x_buf[:-1]
        # BUG: applies b coefficients in reverse order
        y = 0.0
        for i in range(len(self.b)):
            y += self.b[len(self.b) - 1 - i] * self.x_buf[i]
        # Feedback (this part is correct)
        for i in range(len(self.a) - 1):
            y -= self.a[i + 1] * self.y_buf[i]
        # Shift output buffer
        self.y_buf = [y] + self.y_buf[:-1]
        return y

    def reset(self):
        self.x_buf = [0.0] * len(self.b)
        self.y_buf = [0.0] * (len(self.a) - 1)
