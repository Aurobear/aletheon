"""Synthetic ROS 2 node discovery module.

BUG: Uses a fixed 100ms timeout without jitter, causing thundering-herd
re-discovery floods when multiple nodes restart simultaneously.
"""
import time


class NodeDiscovery:
    BASE_TIMEOUT_MS = 100

    def __init__(self):
        self.attempt = 0

    def discover_peers(self) -> list:
        delay_ms = self.BASE_TIMEOUT_MS  # BUG: no backoff, no jitter
        time.sleep(delay_ms / 1000.0)
        self.attempt += 1
        return [f"node_{i}" for i in range(3)]

    def reset(self):
        self.attempt = 0
