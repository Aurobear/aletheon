CREATE TABLE IF NOT EXISTS agent_runs (
    agent_id TEXT PRIMARY KEY,
    root_agent_id TEXT NOT NULL,
    parent_agent_id TEXT,
    process_id TEXT NOT NULL UNIQUE,
    operation_id TEXT NOT NULL UNIQUE,
    runtime_id TEXT NOT NULL,
    profile_id TEXT NOT NULL,
    status TEXT NOT NULL,
    request_json TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    result_json TEXT,
    created_at_ms INTEGER NOT NULL,
    started_at_ms INTEGER,
    ended_at_ms INTEGER,
    last_error TEXT,
    version INTEGER NOT NULL DEFAULT 0,
    retain_until_ms INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_agent_runs_root_status_created
ON agent_runs(root_agent_id, status, created_at_ms DESC);

CREATE INDEX IF NOT EXISTS idx_agent_runs_parent_created
ON agent_runs(parent_agent_id, created_at_ms DESC);

CREATE TABLE IF NOT EXISTS agent_run_messages (
    agent_id TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    from_agent_id TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    content_bytes INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY(agent_id, sequence),
    FOREIGN KEY(agent_id) REFERENCES agent_runs(agent_id)
);
