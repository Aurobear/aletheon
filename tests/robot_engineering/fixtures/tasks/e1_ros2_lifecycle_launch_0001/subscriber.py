"""Synthetic ROS 2 subscriber QoS stub.

The subscription uses best-effort QoS with a zero-length history, so samples
are dropped across a publisher reconnect. The fix is to use a reliable QoS with
a bounded history depth.
"""


class QoSProfile:
    def __init__(self, reliability: str = "best_effort", history_depth: int = 0):
        self.reliability = reliability
        self.history_depth = history_depth


def build_subscription_options() -> QoSProfile:
    # BUG: best-effort with no history drops samples across reconnect.
    return QoSProfile(reliability="best_effort", history_depth=0)
