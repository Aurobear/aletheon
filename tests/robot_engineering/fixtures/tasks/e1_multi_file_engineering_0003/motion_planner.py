"""Motion planner that uses kinematics.

BUG: Imports from a deprecated relative path and calls the old API
compute_ik(joints) which has been replaced by solve_ik(target_pose, seed_joints).
"""

# BUG: deprecated relative import
from .kinematics import compute_ik


def plan_path(start, goal):
    """Plan a path from start to goal."""
    # BUG: old API call
    joints = compute_ik([0.0, 0.0, 0.0])
    return {"path": [start, goal], "joints": joints}
