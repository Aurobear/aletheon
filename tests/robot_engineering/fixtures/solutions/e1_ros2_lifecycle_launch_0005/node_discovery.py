"""Synthetic ROS 2 node discovery module — FIXED.

Uses exponential backoff with random jitter.
"""
import time
import random


class NodeDiscovery:
    BASE_TIMEOUT_MS = 100
    MAX_TIMEOUT_MS = 5000

    def __init__(self):
        self.attempt = 0

    def discover_peers(self) -> list:
        backoff = min(self.BASE_TIMEOUT_MS * (2 ** self.attempt), self.MAX_TIMEOUT_MS)
        jitter = random.uniform(0, backoff * 0.3)
        delay_ms = backoff + jitter
        time.sleep(delay_ms / 1000.0)
        self.attempt += 1
        return [f"node_{i}" for i in range(3)]

    def reset(self):
        self.attempt = 0
