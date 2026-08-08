"""Synthetic ROS 2 subscriber QoS stub - FIXED.

Uses reliable QoS with a bounded history depth so samples are not dropped
across publisher reconnects.
"""


class QoSProfile:
    def __init__(self, reliability: str = "reliable", history_depth: int = 10):
        self.reliability = reliability
        self.history_depth = history_depth


def build_subscription_options() -> QoSProfile:
    return QoSProfile(reliability="reliable", history_depth=10)
