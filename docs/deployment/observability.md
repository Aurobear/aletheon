# Observability and audit retention

The systemd daemon emits newline-delimited JSON to stderr, which journald owns.
Events use stable fields wherever applicable: `component`, `goal_id`,
`attempt_id`, `operation_id`, `event_category`, `duration_ms`, `outcome`, and
`error_code`. Payloads, prompts, email bodies, provider bodies, credentials, and
filesystem secret locations are not observability fields.

Configure journald globally or in a service namespace with a 30-day/2 GiB bound,
rate limiting (for example `RateLimitIntervalSec=30s`, `RateLimitBurst=1000`),
and persistent restricted storage. Vacuum during maintenance and verify recent
boot/error events remain. Retry storms must aggregate counters rather than log
each payload.

Tool audit remains append-only JSONL separate from service logs. Before writing,
the shared Fabric logger redacts sensitive keys and common Authorization,
cookie, token, provider, email, credential-path, and model-key forms; removes
control characters; and bounds untrusted strings. Each record includes
`_previous_hash` and `_record_hash` so continuity across records and restarts can
be verified. Audit files are created `0600` and rotated daily/at 100 MiB with
365 generations. Because `copytruncate` preserves the writer inode but creates
a new chain segment, retain the rotated predecessor when validating the first
post-rotation `_previous_hash`.

Health exposes only bounded categories/counts/ages. Collect queue depth, sync
lag, retry count, free disk/inodes, backup age, worker crash count, and oldest
approval age. Do not deploy a public dashboard or metrics listener; query the
local Unix health RPC through approved Tailscale SSH.

Release tests inject non-production canaries into headers, multiline messages,
email/provider bodies, huge errors, and model-key forms; none may appear in
journal/audit/backup/support output. Also exercise retry-rate limits, rotation
while writing, journal vacuum, daemon restart, and cross-file audit-chain
continuity. Treat a broken chain as an operational incident, not permission to
rewrite historical audit files.
