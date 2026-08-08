"""Synthetic trajectory velocity/acceleration limiter — FIXED.

Applies acceleration limiting before velocity limiting to ensure constraints.
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
        # Step 1: acceleration limit first
        desired_vel = (desired_pos - prev_pos) / dt
        acc = (desired_vel - prev_vel) / dt
        if abs(acc) > max_acc:
            acc = max_acc if acc > 0 else -max_acc
            desired_vel = prev_vel + acc * dt
        # Step 2: velocity limit
        if abs(desired_vel) > max_vel:
            desired_vel = max_vel if desired_vel > 0 else -max_vel
        limited_pos = prev_pos + desired_vel * dt
        result.append(limited_pos)
        prev_pos = limited_pos
        prev_vel = desired_vel
    return result
