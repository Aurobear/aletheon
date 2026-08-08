"""Synthetic packet drop detector — FIXED.

Detects both duplicates and gaps in sequence numbers.
"""
from typing import List, Set


class DropDetector:
    def __init__(self):
        self.seen: Set[int] = set()
        self.duplicate_count = 0
        self.last_seq = None

    def process_packet(self, seq: int) -> int:
        drops = 0
        if seq in self.seen:
            self.duplicate_count += 1
            return 0
        if self.last_seq is not None and seq > self.last_seq + 1:
            drops = seq - self.last_seq - 1
        self.seen.add(seq)
        self.last_seq = seq
        return drops

    def reset(self):
        self.seen.clear()
        self.duplicate_count = 0
        self.last_seq = None
