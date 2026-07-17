pub(crate) const SCHEMA: &str = r#"
PRAGMA foreign_keys=ON;
CREATE TABLE IF NOT EXISTS memory_extraction_jobs(
 id INTEGER PRIMARY KEY, idempotency_key TEXT NOT NULL UNIQUE, session_id TEXT NOT NULL,
 goal_id TEXT, ephemeral INTEGER NOT NULL, memory_worker INTEGER NOT NULL,
 completed_at_ms INTEGER, status TEXT NOT NULL, attempts INTEGER NOT NULL DEFAULT 0,
 lease_owner TEXT, lease_until_ms INTEGER, retry_at_ms INTEGER NOT NULL DEFAULT 0,
 last_error TEXT, watermark TEXT, scope_json TEXT, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS idx_memory_extraction_claim
 ON memory_extraction_jobs(status,retry_at_ms,completed_at_ms);
CREATE TABLE IF NOT EXISTS memory_extraction_events(
 job_id INTEGER NOT NULL REFERENCES memory_extraction_jobs(id),
 event_id TEXT NOT NULL, kind TEXT NOT NULL, content TEXT NOT NULL,
 PRIMARY KEY(job_id,event_id));
CREATE TABLE IF NOT EXISTS memory_candidates(
 id INTEGER PRIMARY KEY, job_id INTEGER NOT NULL REFERENCES memory_extraction_jobs(id),
 candidate_key TEXT NOT NULL UNIQUE, kind_json TEXT NOT NULL, claim TEXT NOT NULL,
 source_event_ids_json TEXT NOT NULL, confidence REAL NOT NULL, scope_json TEXT NOT NULL,
 valid_from_ms INTEGER, valid_until_ms INTEGER, redaction_version INTEGER NOT NULL,
 content_hash TEXT NOT NULL, decision TEXT, decided_record_id TEXT);
CREATE TABLE IF NOT EXISTS memory_records(
 record_id TEXT PRIMARY KEY, candidate_id INTEGER NOT NULL UNIQUE REFERENCES memory_candidates(id),
 scope_json TEXT NOT NULL, kind_json TEXT NOT NULL, content TEXT NOT NULL,
 source_event_ids_json TEXT NOT NULL, content_hash TEXT NOT NULL,
 status TEXT NOT NULL, version INTEGER NOT NULL, created_at_ms INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS memory_scope_leases(
 scope_key TEXT PRIMARY KEY, owner TEXT NOT NULL, lease_until_ms INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS memory_consolidation_runs(
 id INTEGER PRIMARY KEY, scope_key TEXT NOT NULL, owner TEXT NOT NULL,
 candidate_snapshot_json TEXT NOT NULL, watermark TEXT NOT NULL,
 decisions_json TEXT NOT NULL, completed_at_ms INTEGER NOT NULL,
 UNIQUE(scope_key,watermark));
"#;
