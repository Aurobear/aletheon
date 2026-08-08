"""Synthetic ROS 2 topic remapping engine — FIXED.

Correctly handles trailing slashes in node namespace.
"""

class TopicRemapper:
    def __init__(self, node_namespace: str = ""):
        self.ns = node_namespace.rstrip("/")

    def remap(self, topic: str) -> str:
        if topic.startswith("/"):
            return topic
        if self.ns:
            return self.ns + "/" + topic
        return "/" + topic

    def apply_rules(self, topic: str, rules: dict) -> str:
        resolved = self.remap(topic)
        return rules.get(resolved, resolved)
