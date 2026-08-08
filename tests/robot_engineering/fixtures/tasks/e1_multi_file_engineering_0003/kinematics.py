"""Kinematics library v2.

The old compute_ik(joints) API has been replaced by
solve_ik(target_pose, seed_joints).
"""
from typing import List, Tuple
import math


def solve_ik(target_pose: Tuple[float, float, float],
              seed_joints: List[float]) -> List[float]:
    """Solve inverse kinematics for the given target pose.

    Args:
        target_pose: (x, y, theta) in world frame
        seed_joints: initial joint angles as starting guess

    Returns:
        joint angles solution
    """
    # Simple mock: return seed joints adjusted toward target
    x, y, theta = target_pose
    result = []
    for i, j in enumerate(seed_joints):
        result.append(j + 0.1 * (x + y) / (i + 1))
    return result
