-- Durable robot episode store.
-- `attempt_id` is an independent attempt identifier (present even when the
-- underlying operation was never created). `operation_id` is NULL when the
-- operation was never created — never a fabricated id.
-- `INSERT OR IGNORE` on (episode_id, attempt, request_digest) makes replay of
-- an identical attempt a no-op instead of a duplicate side effect.
CREATE TABLE IF NOT EXISTS episodes (
  episode_id        TEXT NOT NULL,
  attempt           INTEGER NOT NULL,
  attempt_id        TEXT NOT NULL,
  operation_id      TEXT,
  request_digest    TEXT NOT NULL,
  status            TEXT NOT NULL,
  expected_json     TEXT NOT NULL,
  before_json       TEXT,
  after_json        TEXT,
  result_json       TEXT,
  verification_json TEXT,
  created_at_ms     INTEGER NOT NULL,
  settled_at_ms     INTEGER,
  PRIMARY KEY (episode_id, attempt, attempt_id)
);

CREATE UNIQUE INDEX IF NOT EXISTS episodes_digest
  ON episodes(episode_id, attempt, request_digest);
