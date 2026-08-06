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
  request_json      TEXT,
  request_digest    TEXT NOT NULL,
  status            TEXT NOT NULL,
  expected_json     TEXT NOT NULL,
  before_sequence   INTEGER,
  after_sequence    INTEGER,
  result_json       TEXT,
  verification_json TEXT,
  verified_sequence INTEGER,
  retry_reason      TEXT,
  created_at_ms     INTEGER NOT NULL,
  settled_at_ms     INTEGER,
  PRIMARY KEY (episode_id, attempt, attempt_id)
);

CREATE UNIQUE INDEX IF NOT EXISTS episodes_digest
  ON episodes(episode_id, attempt, request_digest);

CREATE UNIQUE INDEX IF NOT EXISTS episodes_operation_identity
  ON episodes(operation_id) WHERE operation_id IS NOT NULL;

-- Episode lifecycle is independent of attempt rows so a pre-execution failure
-- (zero attempts) still has a crash-recoverable terminal settlement.
CREATE TABLE IF NOT EXISTS episode_states (
  episode_id       TEXT PRIMARY KEY,
  status           TEXT NOT NULL CHECK(status IN ('running','completed','failed','cancelled')),
  created_at_ms    INTEGER NOT NULL,
  settled_at_ms    INTEGER
);

-- Final report receipts are immutable. The JSON contains bounded metadata and
-- artifact manifests/references only; artifact bytes remain in the artifact
-- store.
CREATE TABLE IF NOT EXISTS episode_reports (
  episode_id              TEXT PRIMARY KEY,
  report_json             TEXT NOT NULL,
  report_sha256           TEXT NOT NULL,
  settlement              TEXT NOT NULL CHECK(settlement IN ('completed','failed','cancelled')),
  settled_at_unix_ms      INTEGER NOT NULL,
  created_at_monotonic_ms INTEGER NOT NULL
);

CREATE TRIGGER IF NOT EXISTS episode_reports_immutable_update
BEFORE UPDATE ON episode_reports
BEGIN
  SELECT RAISE(ABORT, 'settled episode report is immutable');
END;

CREATE TRIGGER IF NOT EXISTS episode_reports_immutable_delete
BEFORE DELETE ON episode_reports
BEGIN
  SELECT RAISE(ABORT, 'settled episode report is immutable');
END;

-- Read-time retention projection. The immutable report receipt never changes;
-- deleting artifact bytes appends one small tombstone retaining URI + digest.
CREATE TABLE IF NOT EXISTS episode_artifact_tombstones (
  episode_id           TEXT NOT NULL REFERENCES episode_reports(episode_id),
  uri                  TEXT NOT NULL,
  digest               TEXT NOT NULL,
  expired_at_unix_ms   INTEGER NOT NULL,
  reason               TEXT NOT NULL,
  PRIMARY KEY (episode_id, digest)
);

CREATE TRIGGER IF NOT EXISTS episode_artifact_tombstones_immutable_update
BEFORE UPDATE ON episode_artifact_tombstones
BEGIN
  SELECT RAISE(ABORT, 'episode artifact tombstone is immutable');
END;

CREATE TRIGGER IF NOT EXISTS episode_artifact_tombstones_immutable_delete
BEFORE DELETE ON episode_artifact_tombstones
BEGIN
  SELECT RAISE(ABORT, 'episode artifact tombstone is immutable');
END;
