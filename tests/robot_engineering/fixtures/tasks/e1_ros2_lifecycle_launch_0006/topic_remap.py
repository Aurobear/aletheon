"""Synthetic ROS 2 topic remapping engine.

BUG: Incorrectly resolves nested namespaces when node namespace contains
trailing slashes, mangling relative topic names.
"""

class TopicRemapper:
    def __init__(self, node_namespace: str = ""):
        self.ns = node_namespace

    def remap(self, topic: str) -> str:
        if topic.startswith("/"):
            return topic
        # BUG: naive join doesn't strip trailing slashes
        if self.ns:
            return self.ns + "/" + topic
        return "/" + topic

    def apply_rules(self, topic: str, rules: dict) -> str:
        resolved = self.remap(topic)
        return rules.get(resolved, resolved)
