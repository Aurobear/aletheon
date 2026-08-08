"""Motion planning package.

BUG: Re-exports a function that no longer exists.
"""

# BUG: deprecated re-export
from .motion_planner import plan_path  # noqa: F401
