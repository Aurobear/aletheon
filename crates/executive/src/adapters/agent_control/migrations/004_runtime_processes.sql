CREATE TABLE IF NOT EXISTS agent_runtime_processes (
    agent_id TEXT PRIMARY KEY,
    process_id TEXT NOT NULL UNIQUE,
    generation INTEGER NOT NULL,
    os_pid INTEGER NOT NULL,
    start_time_ticks INTEGER NOT NULL,
    FOREIGN KEY(agent_id) REFERENCES agent_runs(agent_id)
);
