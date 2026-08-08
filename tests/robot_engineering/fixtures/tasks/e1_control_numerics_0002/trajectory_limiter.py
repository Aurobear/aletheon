"""Synthetic trajectory velocity/acceleration limiter.

BUG: Only applies velocity limits, completely missing acceleration limiting.
"""
from typing import List


def limit_trajectory(positions: List[float], dt: float,
                      max_vel: float, max_acc: float) -> List[float]:
    if len(positions) < 2:
        return list(positions)
    result = [positions[0]]
    prev_pos = positions[0]
    prev_vel = 0.0
    for i in range(1, len(positions)):
        desired_pos = positions[i]
        # BUG: only velocity limiting, acceleration limit ignored
        vel = (desired_pos - prev_pos) / dt
        if abs(vel) > max_vel:
            vel = max_vel if vel > 0 else -max_vel
        limited_pos = prev_pos + vel * dt
        result.append(limited_pos)
        prev_pos = limited_pos
        prev_vel = vel
    return result
