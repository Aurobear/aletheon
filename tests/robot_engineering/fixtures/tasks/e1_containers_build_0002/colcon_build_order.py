"""Synthetic colcon package build order calculator.

BUG: Incorrectly orders packages when there are diamond dependencies
(A->B,C; B->D; C->D), causing D to be built after its consumers.
"""
from collections import defaultdict
from typing import Dict, List, Set


class PackageGraph:
    def __init__(self):
        self.deps: Dict[str, Set[str]] = defaultdict(set)

    def add_package(self, name: str, depends_on: List[str] = None):
        if depends_on:
            self.deps[name].update(depends_on)
        else:
            self.deps.setdefault(name, set())

    def build_order(self) -> List[str]:
        # BUG: simple BFS ignores diamond dependency depth
        visited = set()
        order = []
        def visit(name):
            if name in visited:
                return
            visited.add(name)
            # BUG: visits dependencies after adding current package
            order.append(name)
            for dep in sorted(self.deps.get(name, set())):
                visit(dep)
        for pkg in sorted(self.deps.keys()):
            visit(pkg)
        return order
