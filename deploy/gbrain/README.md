# Local GBrain MCP deployment

This stack runs optional supplemental memory. Local Mnemosyne remains the core
memory service and Aletheon continues locally when this stack is stopped.

```text
Aletheon MemoryService
  ├─ local Mnemosyne (authoritative, synchronous)
  └─ SQLite spool -> HTTP MCP -> GBrain (optional, asynchronous)
```

## Immutable version and contract

Use GBrain release `v0.42.59.0` at commit
`5008b287e47bf791132eedfebf66bdef11e9398c`. Build the image from that checkout
and record its immutable `RepoDigests` value in the operator-owned `.env`;
never deploy by tag or image ID. The captured `tools/list` response is
[`config/gbrain/tools-schema.json`](../../config/gbrain/tools-schema.json).
Aletheon validates the exact required input schemas for `query`, `search`,
`get_page`, and `put_page` before enabling supplemental memory.

Example build from an operator-controlled checkout:

```bash
git -C /path/to/gbrain fetch --tags
git -C /path/to/gbrain checkout --detach 5008b287e47bf791132eedfebf66bdef11e9398c
test "$(git -C /path/to/gbrain rev-parse HEAD)" = 5008b287e47bf791132eedfebf66bdef11e9398c
docker build -t gbrain:5008b287e47bf791132eedfebf66bdef11e9398c /path/to/gbrain
docker image inspect gbrain:5008b287e47bf791132eedfebf66bdef11e9398c \
  --format '{{join .RepoDigests "\n"}}'
```

## Secrets and startup

Copy `.env.example` to `.env`, create distinct least-privilege read and write
token files, and keep both outside Git:

```bash
sudo install -d -o aletheon -g aletheon -m 0750 /var/lib/aletheon/gbrain/{brain,database}
sudo install -m 0600 /dev/null /etc/aletheon/credentials/gbrain-read.token
sudo install -m 0600 /dev/null /etc/aletheon/credentials/gbrain-write.token
# Populate through the local secret manager, not shell history.
scripts/aletheon.sh verify compose deploy/gbrain/.env
docker compose --env-file deploy/gbrain/.env \
  -f deploy/gbrain/compose.yaml -f deploy/compose.production.yaml up -d
```

The compose file mounts secret files rather than embedding values. If the
pinned GBrain build is placed behind an authentication proxy, configure that
proxy to map each token to only its intended source and operation set. Aletheon
uses `GBRAIN_TOKEN` from its own runtime secret environment; do not copy tokens
into TOML. Use separate Aletheon instances/tokens when read and write grants
must be strictly separated. The MCP port binds to loopback by default and the
container also uses an internal network.

## MCP validation and source binding

Run these against a disposable deployment before enabling Aletheon:

```bash
endpoint=http://127.0.0.1:9020/mcp
curl -fsS -X POST "$endpoint" -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{}}}'
curl -fsS -X POST "$endpoint" -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' > /tmp/gbrain-tools.json
cargo test -p mnemosyne -- backends::gbrain::schema
cargo test -p executive --test gbrain_mcp_adapter
```

`query` receives an explicit configured `source_id`. `search` is scoped by MCP
token context. `get_page` and `put_page` have no source argument, so their token
must be bound to the intended source. A schema mismatch is a release failure,
not a reason to bypass validation.

Configure a local Aletheon override (not `config/default.toml`):

```toml
[memory.gbrain]
enabled = true
projection_enabled = true
server_name = "gbrain"
read_sources = ["aletheon", "general"]
write_source = "aletheon"

[[mcp_servers]]
name = "gbrain"
transport = "http"
url = "http://127.0.0.1:9020/mcp"
bearer_token_env = "GBRAIN_TOKEN"
```

## Health and spool operations

```bash
docker compose --env-file deploy/gbrain/.env -f deploy/gbrain/compose.yaml -f deploy/compose.production.yaml ps
docker compose --env-file deploy/gbrain/.env -f deploy/gbrain/compose.yaml -f deploy/compose.production.yaml logs --tail=200 gbrain
sqlite3 ~/.aletheon/memory/gbrain-spool.db \
  'select record_id,state,attempts,next_attempt_ms,lease_until_ms from gbrain_queue order by updated_ms;'
sqlite3 ~/.aletheon/memory/gbrain-spool.db \
  'select record_id,attempts,reason_category,failed_ms from gbrain_dead_letters order by failed_ms;'
```

After correcting the cause of a dead letter, use the tested application requeue
path; do not edit queue rows directly. On first M8 startup, bounded legacy JSON
entries from `~/.aletheon/gbrain-outbox` are sanitized and migrated into SQLite,
then the old directory is renamed. Preserve it until migration is audited.

## Backup and restore

Stop writers or take storage-consistent snapshots. Back up all three durable
components together:

1. `/var/lib/aletheon/gbrain/brain` bind volume (Markdown/frontmatter);
2. `/var/lib/aletheon/gbrain/database` bind volume (index/database state);
3. `~/.aletheon/memory/gbrain-spool.db` plus `-wal`/`-shm` while live.

```bash
docker compose --env-file deploy/gbrain/.env -f deploy/gbrain/compose.yaml -f deploy/compose.production.yaml stop gbrain
tar czf backups/gbrain-brain.tgz -C /var/lib/aletheon/gbrain/brain .
tar czf backups/gbrain-database.tgz -C /var/lib/aletheon/gbrain/database .
cp ~/.aletheon/memory/gbrain-spool.db* backups/
```

Restore into empty volumes while both services are stopped, restore the SQLite
files with mode `0600`, start GBrain, validate `initialize`/`tools/list`, then
start Aletheon. Confirm queued record IDs have one receipt or one pending item.

## Upgrade, rollback, and full disable

For an upgrade, build a new immutable image, capture its live `tools/list` into
a review branch, compare required schemas, back up data, and only then change
the image and `schema_version`. Never reuse the old tag.

To disable completely, set both `enabled = false` and
`projection_enabled = false`, remove/comment the `gbrain` MCP server override,
and restart Aletheon. Local Mnemosyne recall and Goal execution remain active.
The SQLite spool and GBrain volumes are retained for later recovery; deleting
them is a separate destructive operator action.
