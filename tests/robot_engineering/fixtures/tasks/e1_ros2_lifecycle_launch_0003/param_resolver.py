"""Synthetic ROS 2 launch parameter resolver.

BUG: Applies overrides in insertion order instead of priority order.
Correct priority: CLI args > launch file > defaults.
The buggy version reverses CLI and launch priority.
"""

class ParamResolver:
    def __init__(self, defaults: dict = None):
        self.defaults = dict(defaults or {})
        self.launch_overrides = {}
        self.cli_overrides = {}

    def add_launch_override(self, key: str, value):
        self.launch_overrides[key] = value

    def add_cli_override(self, key: str, value):
        self.cli_overrides[key] = value

    def resolve(self, key: str):
        # BUG: launch overrides CLI (wrong priority)
        val = self.defaults.get(key)
        if key in self.cli_overrides:
            val = self.cli_overrides[key]
        if key in self.launch_overrides:
            val = self.launch_overrides[key]
        return val

    def resolve_all(self) -> dict:
        result = dict(self.defaults)
        result.update(self.cli_overrides)
        result.update(self.launch_overrides)
        return result
