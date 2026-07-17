CREATE TABLE IF NOT EXISTS agent_resource_leases (
    lease_key TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('admission','mailbox','execution','worktree')),
    owner TEXT NOT NULL,
    expires_at_ms INTEGER NOT NULL,
    worktree_root TEXT,
    worktree_path TEXT,
    expected_head TEXT,
    FOREIGN KEY(agent_id) REFERENCES agent_runs(agent_id)
);

CREATE INDEX IF NOT EXISTS idx_agent_resource_leases_expiry
ON agent_resource_leases(expires_at_ms, lease_key);
