"""Synthetic angle unit conversion module — FIXED.

Uses math.pi for precise conversions.
"""
import math

DEG_PER_RAD = 180.0 / math.pi


def rad_to_deg(rad: float) -> float:
    return rad * DEG_PER_RAD


def deg_to_rad(deg: float) -> float:
    return deg / DEG_PER_RAD


def normalize_angle_deg(deg: float) -> float:
    deg = deg % 360
    if deg >= 180:
        deg -= 360
    return deg
