CREATE TABLE IF NOT EXISTS agent_messages_v2 (
    agent_id TEXT NOT NULL,
    delivery_id TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    from_agent_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    payload_ref TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    delivery_state TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY(agent_id, sequence),
    UNIQUE(agent_id, delivery_id),
    FOREIGN KEY(agent_id) REFERENCES agent_runs(agent_id)
);

CREATE INDEX IF NOT EXISTS idx_agent_messages_v2_delivery
ON agent_messages_v2(agent_id, delivery_id);
