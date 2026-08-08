"""Synthetic colcon package build order calculator — FIXED.

Correctly orders packages with diamond dependencies using topological sort.
"""
from collections import defaultdict
from typing import Dict, List, Set


class PackageGraph:
    def __init__(self):
        self.deps: Dict[str, Set[str]] = defaultdict(set)
        self.reverse_deps: Dict[str, Set[str]] = defaultdict(set)

    def add_package(self, name: str, depends_on: List[str] = None):
        if depends_on:
            for dep in depends_on:
                self.deps[name].add(dep)
                self.reverse_deps[dep].add(name)
        else:
            self.deps.setdefault(name, set())

    def build_order(self) -> List[str]:
        # Kahn's algorithm: start with packages with no dependencies
        in_degree = {}
        for pkg in self.deps:
            in_degree[pkg] = len(self.deps[pkg])
        queue = [p for p, deg in in_degree.items() if deg == 0]
        order = []
        while queue:
            pkg = queue.pop(0)
            order.append(pkg)
            for dependent in sorted(self.reverse_deps.get(pkg, set())):
                in_degree[dependent] -= 1
                if in_degree[dependent] == 0:
                    queue.append(dependent)
        return order
