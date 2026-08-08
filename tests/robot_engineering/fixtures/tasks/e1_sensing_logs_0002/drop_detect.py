"""Synthetic packet drop detector for sensor streams.

BUG: Only checks for duplicate sequence numbers but not for gaps.
A gap of 5 missing packets between seq 10 and 16 should be detected as 5 drops.
"""
from typing import List, Set


class DropDetector:
    def __init__(self):
        self.seen: Set[int] = set()
        self.duplicate_count = 0
        self.last_seq = None

    def process_packet(self, seq: int) -> int:
        """Process a packet sequence number. Returns number of drops detected."""
        drops = 0
        if seq in self.seen:
            self.duplicate_count += 1
            return 0  # duplicate, not a drop
        # BUG: doesn't check for gaps
        self.seen.add(seq)
        self.last_seq = seq
        return drops

    def reset(self):
        self.seen.clear()
        self.duplicate_count = 0
        self.last_seq = None
