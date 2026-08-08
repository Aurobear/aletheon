"""Durable run, occurrence and failure-cluster index."""

from __future__ import annotations

import json
import sqlite3
from collections.abc import Mapping
from pathlib import Path

_SCHEMA = """
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS schema_info (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    version INTEGER NOT NULL
);
INSERT OR IGNORE INTO schema_info(singleton, version) VALUES (1, 1);

CREATE TABLE IF NOT EXISTS runs (
    run_id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT NOT NULL,
    outcome TEXT NOT NULL,
    failure_class TEXT NOT NULL,
    fingerprint TEXT,
    bundle_path TEXT NOT NULL,
    result_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS runs_case_finished ON runs(case_id, finished_at DESC);

CREATE TABLE IF NOT EXISTS clusters (
    fingerprint TEXT PRIMARY KEY,
    case_id TEXT NOT NULL,
    failure_class TEXT NOT NULL,
    first_seen TEXT NOT NULL,
    last_seen TEXT NOT NULL,
    occurrence_count INTEGER NOT NULL CHECK (occurrence_count > 0),
    status TEXT NOT NULL,
    latest_run_id TEXT NOT NULL REFERENCES runs(run_id)
);

CREATE TABLE IF NOT EXISTS occurrences (
    run_id TEXT PRIMARY KEY REFERENCES runs(run_id),
    fingerprint TEXT NOT NULL REFERENCES clusters(fingerprint),
    observed_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS occurrences_fingerprint
    ON occurrences(fingerprint, observed_at DESC);

CREATE TABLE IF NOT EXISTS diagnostic_calls (
    run_id TEXT PRIMARY KEY,
    requested_at TEXT NOT NULL,
    request_day TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS diagnostic_calls_day ON diagnostic_calls(request_day);
"""


class LabStore:
    def __init__(self, path: Path):
        path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.path = path
        self.connection = sqlite3.connect(path, timeout=30.0)
        self.connection.row_factory = sqlite3.Row
        self.connection.execute("PRAGMA busy_timeout = 30000")
        self.connection.executescript(_SCHEMA)
        version = self.connection.execute(
            "SELECT version FROM schema_info WHERE singleton = 1"
        ).fetchone()
        if version is None or version["version"] != 1:
            raise RuntimeError("unsupported Nightwatch database schema")

    def close(self) -> None:
        self.connection.close()

    def __enter__(self) -> LabStore:  # noqa: PYI034
        return self

    def __exit__(self, *_: object) -> None:
        self.close()

    def record(
        self, result: Mapping[str, object], bundle_path: Path
    ) -> dict[str, object]:
        run_id = str(result["run_id"])
        fingerprint = result.get("fingerprint")
        serialized = json.dumps(result, sort_keys=True, separators=(",", ":"))
        with self.connection:
            self.connection.execute(
                """
                INSERT INTO runs(
                    run_id, case_id, commit_sha, started_at, finished_at,
                    outcome, failure_class, fingerprint, bundle_path, result_json
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    run_id,
                    result["case_id"],
                    result["source"]["commit_sha"],
                    result["started_at"],
                    result["finished_at"],
                    result["outcome"],
                    result["failure_class"],
                    fingerprint,
                    str(bundle_path),
                    serialized,
                ),
            )
            if not isinstance(fingerprint, str):
                return {"is_new": False, "occurrence_count": 0, "fingerprint": None}
            existing = self.connection.execute(
                "SELECT occurrence_count FROM clusters WHERE fingerprint = ?",
                (fingerprint,),
            ).fetchone()
            is_new = existing is None
            if is_new:
                self.connection.execute(
                    """
                    INSERT INTO clusters(
                        fingerprint, case_id, failure_class, first_seen, last_seen,
                        occurrence_count, status, latest_run_id
                    ) VALUES (?, ?, ?, ?, ?, 1, 'new', ?)
                    """,
                    (
                        fingerprint,
                        result["case_id"],
                        result["failure_class"],
                        result["finished_at"],
                        result["finished_at"],
                        run_id,
                    ),
                )
                count = 1
            else:
                self.connection.execute(
                    """
                    UPDATE clusters
                       SET last_seen = ?, occurrence_count = occurrence_count + 1,
                           latest_run_id = ?
                     WHERE fingerprint = ?
                    """,
                    (result["finished_at"], run_id, fingerprint),
                )
                count = int(existing["occurrence_count"]) + 1
            self.connection.execute(
                "INSERT INTO occurrences(run_id, fingerprint, observed_at) VALUES (?, ?, ?)",
                (run_id, fingerprint, result["finished_at"]),
            )
        return {
            "is_new": is_new,
            "occurrence_count": count,
            "fingerprint": fingerprint,
        }

    def latest_finished_at(self, case_id: str) -> str | None:
        row = self.connection.execute(
            "SELECT finished_at FROM runs WHERE case_id = ? ORDER BY finished_at DESC LIMIT 1",
            (case_id,),
        ).fetchone()
        return str(row["finished_at"]) if row else None

    def cluster(self, fingerprint: str) -> dict[str, object] | None:
        row = self.connection.execute(
            "SELECT * FROM clusters WHERE fingerprint = ?", (fingerprint,)
        ).fetchone()
        return dict(row) if row else None

    def reserve_diagnostic_call(
        self, *, run_id: str, requested_at: str, max_calls_per_day: int
    ) -> bool:
        request_day = requested_at[:10]
        with self.connection:
            count = self.connection.execute(
                "SELECT COUNT(*) AS value FROM diagnostic_calls WHERE request_day = ?",
                (request_day,),
            ).fetchone()["value"]
            if int(count) >= max_calls_per_day:
                return False
            self.connection.execute(
                "INSERT INTO diagnostic_calls(run_id, requested_at, request_day) VALUES (?, ?, ?)",
                (run_id, requested_at, request_day),
            )
        return True
