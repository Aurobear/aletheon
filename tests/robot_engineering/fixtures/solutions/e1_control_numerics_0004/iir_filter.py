"""Synthetic IIR (Infinite Impulse Response) filter — FIXED.

Applies feedforward coefficients b in the correct forward order.
"""
from typing import List


class IIRFilter:
    def __init__(self, b: List[float], a: List[float]):
        self.b = list(b)
        self.a = list(a)
        self.x_buf = [0.0] * len(b)
        self.y_buf = [0.0] * (len(a) - 1)

    def process(self, x: float) -> float:
        self.x_buf = [x] + self.x_buf[:-1]
        y = 0.0
        for i in range(len(self.b)):
            y += self.b[i] * self.x_buf[i]
        for i in range(len(self.a) - 1):
            y -= self.a[i + 1] * self.y_buf[i]
        self.y_buf = [y] + self.y_buf[:-1]
        return y

    def reset(self):
        self.x_buf = [0.0] * len(self.b)
        self.y_buf = [0.0] * (len(self.a) - 1)
