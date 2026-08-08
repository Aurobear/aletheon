"""Synthetic ROS 2 bag index parser.

BUG: When chunk headers are corrupted (negative offset or impossibly large size),
the parser returns incorrect positions instead of detecting and skipping.
"""
from dataclasses import dataclass
from typing import List


@dataclass
class ChunkEntry:
    chunk_id: int
    offset: int
    size: int
    message_count: int


class BagIndexParser:
    MAX_CHUNK_SIZE = 100_000_000

    def __init__(self):
        self.entries: List[ChunkEntry] = []

    def add_raw_chunk(self, chunk_id: int, offset: int, size: int, msg_count: int) -> bool:
        # BUG: no validation — accepts corrupted entries
        self.entries.append(ChunkEntry(chunk_id, offset, size, msg_count))
        return True

    def get_valid_entries(self) -> List[ChunkEntry]:
        return list(self.entries)
