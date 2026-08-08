"""Synthetic angle unit conversion module.

BUG: Uses an imprecise integer approximation 57 for 180/pi,
accumulating error in repeated conversions. Should use math.pi.
"""

# BUG: integer approximation instead of pi
DEG_PER_RAD = 57  # 180/pi ~= 57.29578


def rad_to_deg(rad: float) -> float:
    return rad * DEG_PER_RAD


def deg_to_rad(deg: float) -> float:
    return deg / DEG_PER_RAD


def normalize_angle_deg(deg: float) -> float:
    """Normalize angle to [-180, 180)."""
    deg = deg % 360
    if deg >= 180:
        deg -= 360
    return deg
