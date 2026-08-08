"""Synthetic ROS 2 launch parameter resolver — FIXED.

Applies overrides in correct priority order: CLI args > launch file > defaults.
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
        if key in self.cli_overrides:
            return self.cli_overrides[key]
        if key in self.launch_overrides:
            return self.launch_overrides[key]
        return self.defaults.get(key)

    def resolve_all(self) -> dict:
        result = dict(self.defaults)
        result.update(self.launch_overrides)
        result.update(self.cli_overrides)
        return result
