"""Synthetic math utility library.

Functions: clamp, lerp, normalize_angle.
"""
import math


def clamp(value: float, lo: float, hi: float) -> float:
    """Clamp value between lo and hi."""
    return max(lo, min(hi, value))


def lerp(a: float, b: float, t: float) -> float:
    """Linear interpolation between a and b."""
    return a + (b - a) * t


def normalize_angle(rad: float) -> float:
    """Normalize angle to [-pi, pi)."""
    rad = rad % (2 * math.pi)
    if rad >= math.pi:
        rad -= 2 * math.pi
    return rad
