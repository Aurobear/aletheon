# Production container boundary

Aletheon Executive runs natively under systemd. Only optional GBrain/database
storage is containerized; internal agents remain Executive-managed.

```text
native Aletheon -> 127.0.0.1:9020 -> non-root GBrain container
                                      ├─ internal-only network
                                      ├─ /var/lib/aletheon/gbrain/brain
                                      └─ /var/lib/aletheon/gbrain/database
```

Use both Compose files and an operator-owned environment file:

```bash
scripts/verify-compose.sh deploy/gbrain/.env
docker compose --env-file deploy/gbrain/.env \
  -f deploy/gbrain/compose.yaml -f deploy/compose.production.yaml up -d
```

The image reference is `repository@sha256:digest`; tags, `latest`, and image IDs
are rejected. The compatible application contract is GBrain `v0.42.59.0`,
commit `5008b287e47bf791132eedfebf66bdef11e9398c`, MCP protocol `2024-11-05`, and
the checked-in `config/gbrain/tools-schema.json` required-operation fixture.

The only published endpoint is loopback. Host firewall validation must show no
listener on LAN/WAN addresses. The container is non-root, capability-free,
PID/resource limited, read-only except bind-backed data and bounded tmpfs, and
uses read-only secret mounts plus bounded JSON logs.

Before an upgrade:

1. stop intake and create a verified backup;
2. build the new commit and record its immutable RepoDigest;
3. capture and compare `tools/list`, then review migrations;
4. start with disposable copied data and run health/contract tests;
5. update the digest, start production, and retain the pre-upgrade backup.

Rollback the binary/image only when its schema is compatible. If migration is
not backward-compatible, stop services and restore the matching pre-upgrade
data first. Never run an older image directly against migrated storage.
