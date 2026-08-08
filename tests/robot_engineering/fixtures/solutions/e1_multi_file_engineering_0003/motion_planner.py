"""Motion planner — FIXED.

Uses new solve_ik API from kinematics v2.
"""
from kinematics import solve_ik


def plan_path(start, goal):
    """Plan a path from start to goal."""
    joints = solve_ik((start[0], start[1], 0.0), [0.0, 0.0, 0.0])
    return {"path": [start, goal], "joints": joints}
