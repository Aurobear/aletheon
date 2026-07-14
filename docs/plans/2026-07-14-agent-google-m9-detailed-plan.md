# Aletheon M9 Home Deployment Hardening Detailed Plan

> **For agentic workers:** Treat deployment assets as executable contracts. Validate installation, boot, failure, backup, and restore on a disposable Ubuntu host before changing the production machine.

**Goal:** Run the enabled Aletheon milestones continuously on one private Linux server with least privilege, restart recovery, private administration, bounded storage, observable health, and tested encrypted backups.

**Architecture:** Run one native Aletheon Executive under systemd; Executive continues supervising internal agents, Telegram, and Google sync. Run optional GBrain dependencies in pinned containers. Expose only authenticated Unix sockets locally and administer over Tailscale/SSH; no public HTTP endpoint is introduced.

**Tech Stack:** Ubuntu Server LTS/Debian Stable, systemd, Docker Compose for GBrain, Tailscale, Unix sockets, journald/logrotate, SQLite backup APIs, restic-compatible encrypted backup tooling.

---

## 1. Requirement and code anchors

- Target is a one-user continuously running deployment on Linux with unattended boot, restart, health, encrypted secrets, and backups: `docs/arch/agent-google/01_HOME_DEPLOYMENT_ARCHITECTURE.md:1-18` and `:44-59`.
- Executive is native while GBrain/PostgreSQL may be containerized; internal workers remain Executive-managed: `docs/arch/agent-google/01_HOME_DEPLOYMENT_ARCHITECTURE.md:60-88` and `:106-129`.
- Private administration uses Tailscale; Telegram polling requires no inbound public port: `docs/arch/agent-google/01_HOME_DEPLOYMENT_ARCHITECTURE.md:90-104`.
- Storage, secret, and backup requirements are at `docs/arch/agent-google/01_HOME_DEPLOYMENT_ARCHITECTURE.md:131-172`.
- Deployment acceptance is headless boot, auto-restart, durable Goals, crash isolation, no leaked secrets, restore, and Tailscale administration: `docs/arch/agent-google/01_HOME_DEPLOYMENT_ARCHITECTURE.md:174-186`.
- M9 explicitly requires systemd, Compose, backups, Tailscale, secret management, health, log rotation, and disk quotas: `docs/arch/agent-google/05_IMPLEMENTATION_ROADMAP.md:178-188`.
- Existing system unit already uses a dedicated user, StateDirectory/RuntimeDirectory, restart, and NoNewPrivileges: `config/aletheon.service:1-21`.
- Current daemon listens on a credential-checked mode-0660 Unix socket: `crates/executive/src/impl/daemon/server.rs:20-65`.
- Current health RPC reports only process-level uptime/connections/session count: `crates/executive/src/impl/daemon/handler/rpc/rpc_health.rs:93-115`.

## 2. Task 1 — Finalize filesystem and configuration contract

**Files:**

- Modify: `crates/fabric/src/types/paths.rs`
- Modify: `config/default.toml`
- Create: `config/production.toml.example`
- Create: `docs/deployment/filesystem-layout.md`
- Test: path/config tests in existing modules

- [ ] Define canonical production roots: `/var/lib/aletheon`, `/etc/aletheon`, `/run/aletheon`, `/var/cache/aletheon`, and their state/goals/sessions/mnemosyne/artifacts/worktrees/audit subdirectories.
- [ ] Remove split `/var/run` versus `/run` assumptions by resolving one canonical runtime path while retaining compatible symlink behavior supplied by Linux.
- [ ] Add typed config for each data path, quota, enabled integration, secret-file reference, backup mode, and health threshold; environment variables may override non-secret values only where documented.
- [ ] Refuse relative/world-writable production paths, symlinked secret roots, data paths outside approved roots, and unresolved `~` under the service account.
- [ ] Document ownership/modes: config/policy root-owned readable by service as needed, credential keys `0600`, state `0750`, runtime socket directory `0750`, socket `0660`.
- [ ] Test production/default/user modes, path traversal/symlink cases, missing directories, and backwards-compatible development paths.
- [ ] Run scoped Fabric/config tests; expect PASS.
- [ ] Commit `feat(config): define production filesystem layout`.

## 3. Task 2 — Harden the systemd service and installer

**Files:**

- Modify: `config/aletheon.service`
- Preserve: `config/aletheon.user.service` for development only
- Create: `scripts/install-systemd.sh`
- Create: `scripts/verify-systemd.sh`
- Create: `docs/deployment/systemd.md`

- [ ] Add `ExecStartPre` configuration/permission validation and a readiness probe; use the actual daemon CLI/config flags verified from `--help`, not assumed flags.
- [ ] Add graceful stop timeout and kill policy, start-rate limiting, watchdog only after daemon heartbeat support exists, and `Restart=on-failure`.
- [ ] Apply supported hardening after sandbox/worktree tests: `PrivateTmp`, `ProtectSystem=strict`, `ProtectHome`, explicit `ReadWritePaths`, `RestrictSUIDSGID`, capability bounding, syscall/address-family restrictions, and `UMask=0027`.
- [ ] Preserve namespace/bubblewrap requirements for Pi; if a hardening directive breaks required isolation, document the minimal exception rather than disabling sandboxing.
- [ ] Installer creates system user/group/directories atomically, copies only versioned assets, validates permissions, runs daemon config check, then enables service. Re-run is idempotent.
- [ ] Do not embed provider, Telegram, Google, backup, or GBrain secrets in the unit or environment visible through `systemctl show`; reference credential files.
- [ ] Validate with `systemd-analyze verify`, shellcheck, clean install, second install, bad config, SIGTERM during active Goal, crash restart, and boot enablement.
- [ ] Commit `chore(deploy): harden Aletheon systemd service`.

## 4. Task 3 — Add production health and readiness

**Files:**

- Modify: `crates/executive/src/impl/daemon/handler/rpc/rpc_health.rs`
- Create: `crates/executive/src/impl/health.rs`
- Create: `scripts/aletheon-healthcheck.sh`
- Test: `crates/executive/tests/production_health.rs`

- [ ] Separate liveness (event loop responds) from readiness (stores opened, migrations complete, Goal coordinator recovered, channel state known).
- [ ] Report sanitized component states for ObjectiveStore, approvals/outboxes, Telegram, Google sync/auth, worktree capacity, disk/quota, local memory, GBrain/spool, and last backup marker.
- [ ] Classify OptionalDegraded versus RequiredUnready so GBrain/Google outages do not mark core daemon dead while a failed Goal database does.
- [ ] Health output contains counts/timestamps/error categories only—no messages, tokens, addresses, filesystem secrets, or provider response bodies.
- [ ] Healthcheck connects through the local credential-checked Unix socket and exits 0/1/2 for ready/degraded/unready.
- [ ] Test startup recovery, corrupt required database, optional outage, full disk, stale sync, overdue backup, shutdown, unauthorized socket peer, and redaction snapshots.
- [ ] Run `cargo test -p executive --test production_health`; expect PASS.
- [ ] Commit `feat(executive): expose deployment readiness health`.

## 5. Task 4 — Pin and isolate container services

**Files:**

- Modify/Create: `deploy/gbrain/compose.yaml` from M8
- Create: `deploy/compose.production.yaml`
- Create: `deploy/README.md`
- Test: `scripts/verify-compose.sh`

- [ ] Pin every image by digest and record compatible schema/API versions; no `latest` tags.
- [ ] Put GBrain/database on an internal network, bind any host diagnostic port to loopback only, and publish no Internet-facing port.
- [ ] Use named bind-mounted data under `/var/lib/aletheon/gbrain`, read-only secret mounts, non-root containers, read-only root filesystems where supported, dropped capabilities, resource/PID limits, healthchecks, restart policy, and log rotation.
- [ ] Aletheon native service reaches GBrain through loopback/private connector only; firewall tests prove LAN/public interfaces cannot connect.
- [ ] Define ordered upgrade/migration/rollback with pre-upgrade backup. Never downgrade a schema without a verified restore.
- [ ] Run `docker compose config`, container security inspection, disposable-volume health, restart, resource-limit, upgrade, and rollback tests.
- [ ] Commit `chore(deploy): isolate optional container services`.

## 6. Task 5 — Establish secret lifecycle

**Files:**

- Create: `scripts/aletheon-secret-init.sh`
- Create: `scripts/aletheon-secret-audit.sh`
- Create: `docs/deployment/secrets.md`
- Modify: `.gitignore`

- [ ] Inventory model provider, Telegram, Google OAuth/client/vault, GBrain, backup repository, and Tailscale/host credentials with owner, path, rotation, and revocation procedure.
- [ ] Generate credential-vault master key using kernel CSPRNG into a root-created `0600` file without printing it or passing it in argv.
- [ ] Validate owner/mode, regular-file/no-symlink, parent permissions, and absence from repository before service start.
- [ ] Rotation procedures support overlap where provider permits, atomic secret-file replace, daemon reload/restart, validation, then old credential revocation.
- [ ] Audit git tracked files, unit/environment display, process argv/environ accessibility, journald, backups, crash dumps, and model transcripts using non-secret canary values.
- [ ] Disable core dumps or route encrypted restricted dumps; ensure support bundles redact known header/token/email patterns.
- [ ] Run shellcheck and secret-canary audit; expect no canary outside approved encrypted store/in-memory process scope.
- [ ] Commit `chore(security): document deployment secret lifecycle`.

## 7. Task 6 — Implement consistent encrypted backups

**Files:**

- Create: `scripts/backup-aletheon.sh`
- Create: `scripts/restore-aletheon.sh`
- Create: `config/aletheon-backup.service`
- Create: `config/aletheon-backup.timer`
- Create: `docs/deployment/backup-restore.md`

- [ ] Produce a manifest with Aletheon/schema versions, UTC time, host ID, included components, per-file hashes, and encryption repository snapshot ID.
- [ ] Snapshot SQLite databases through SQLite online backup/VACUUM INTO or a coordinated quiesce/checkpoint—not raw copying live WAL files independently.
- [ ] Include Goal/approval/channel/Google cursor databases, local memory/GBrain export, artifacts, audit, config/policy, encrypted credential vault, and key references according to the recovery threat model.
- [ ] Keep vault key recovery material separately encrypted from the primary backup repository; document loss/compromise consequences.
- [ ] Use daily local and weekly encrypted remote snapshots with retention/prune/check and failure notifications; backup command logs paths/counts only.
- [ ] Restore refuses a nonempty target unless explicit safe staging is chosen, verifies manifest/hashes/schema, restores ownership/modes, runs offline integrity checks, then starts readiness validation.
- [ ] Test active-Goal backup, WAL consistency, network failure, partial snapshot, wrong key, tamper, empty-host restore, and rollback to the pre-restore directory.
- [ ] Validate timer with `systemd-analyze calendar`; run disposable backup and restore smoke tests.
- [ ] Commit `chore(deploy): add encrypted backup and restore`.

## 8. Task 7 — Enforce storage quotas and cleanup policy

**Files:**

- Create: `crates/executive/src/impl/storage_quota.rs`
- Modify: Goal/artifact/worktree/Google/GBrain spool services selected by prior milestones
- Create: `config/aletheon-cleanup.service`
- Create: `config/aletheon-cleanup.timer`
- Create: `docs/deployment/storage-policy.md`
- Test: `crates/executive/tests/storage_quota.rs`

- [ ] Configure soft/hard byte and item limits per artifacts, worktrees, audit, sessions, Google projection/content, GBrain spool/dead letters, and total data root.
- [ ] Admission checks reserve expected bytes before new Goal attempt/download/artifact; hard-limit failure is explicit and never deletes active/unapproved evidence.
- [ ] Cleanup removes only policy-expired managed data in order: abandoned verified-clean worktrees, caches, old acknowledged outbox/provider projections, expired session excerpts, then retained artifacts according to policy.
- [ ] Preserve active Goal/approval artifacts, latest successful backup metadata, audit minimum retention, and legal/user pins.
- [ ] Use filesystem/device quotas as a second boundary where available; document inode and free-space thresholds.
- [ ] Test concurrent reservations, crash/release, full bytes/inodes, symlinks/hardlinks, active worktree, pinned artifact, cleanup restart, and low-space health transition.
- [ ] Run `cargo test -p executive --test storage_quota`; expect PASS.
- [ ] Commit `feat(executive): enforce managed storage quotas`.

## 9. Task 8 — Configure structured logs and audit retention

**Files:**

- Modify: daemon tracing/bootstrap files selected by current logging initialization
- Create: `config/aletheon.logrotate`
- Create: `docs/deployment/observability.md`
- Test: logging redaction tests

- [ ] Standardize structured fields for component, Goal/attempt/operation IDs, event category, duration, outcome, and stable error code; omit sensitive payloads by default.
- [ ] Centralize redaction for Authorization/cookie/token values, email bodies, provider payloads, filesystem secrets, and model credentials before formatting.
- [ ] Use journald for service logs with configured size/retention/rate limits; rotate file-based append-only audit separately with restrictive permissions and integrity/hash chaining if required by current audit contract.
- [ ] Emit metrics/health counters for queue depth, sync lag, retries, disk, backup age, worker crashes, and approval age without deploying a public dashboard.
- [ ] Test canary secrets, multiline/control characters, huge errors, retry storms/rate limiting, rotation while writing, journal vacuum, and audit continuity.
- [ ] Commit `chore(observability): bound deployment logs and audit`.

## 10. Task 9 — Restrict private administration with Tailscale

**Files:**

- Create: `docs/deployment/tailscale.md`
- Create: `scripts/verify-network-exposure.sh`
- Modify: host firewall example/config documentation only

- [ ] Keep daemon/MCP/health on local Unix sockets; use SSH over Tailscale for administration and file transfer.
- [ ] Define tailnet ACL/tag policy for the owner's devices and server, device approval/expiry, lost-device revocation, and recovery access.
- [ ] Firewall denies unsolicited inbound LAN/WAN traffic; allow established outbound HTTPS/DNS/NTP and Tailscale requirements. Telegram remains long polling.
- [ ] Do not expose OAuth callback publicly by default; use loopback callback plus authenticated operator flow or a documented short-lived Tailscale-only callback listener.
- [ ] Test from localhost, owner tailnet device, unapproved tailnet device, LAN peer, and external interface with `ss`/firewall/Tailscale status evidence.
- [ ] Commit `docs(deploy): define private administration boundary`.

## 11. Task 10 — Build upgrade, rollback, and disaster runbooks

**Files:**

- Create: `scripts/upgrade-aletheon.sh`
- Create: `docs/deployment/upgrade-rollback.md`
- Create: `docs/deployment/disaster-recovery.md`
- Create: `docs/deployment/operations-checklist.md`

- [ ] Upgrade stages verified binary/config/assets, creates backup, stops intake gracefully, applies forward migrations, starts, waits readiness, and records version receipt.
- [ ] Rollback uses the matching binary plus pre-upgrade restored data when schemas are not backward-compatible; never blindly run an old binary on migrated databases.
- [ ] Cover host loss, SSD failure, corrupted Goal DB, lost Telegram/Google credential, compromised vault key, GBrain loss, stuck worker, full disk, and Tailscale loss.
- [ ] Define RPO/RTO targets and manual escalation/notification steps; assumptions must be measured in the release drill.
- [ ] Scripts are fail-fast, idempotent where safe, quote paths, avoid secrets in argv, and never delete the only known-good backup.
- [ ] Run shellcheck and disposable upgrade/rollback/disaster drills.
- [ ] Commit `chore(deploy): add operations recovery runbooks`.

## 12. M9 full acceptance drill

- [ ] Provision a clean supported Ubuntu Server VM using only checked-in docs/assets plus separately supplied secrets.
- [ ] Boot headless and prove systemd starts Aletheon, readiness succeeds, Telegram responds, and no public listener exists.
- [ ] Create a Goal, start a worker, disconnect phone, kill worker, restart daemon, and prove Goal/attempt/approval/outbox recovery.
- [ ] If enabled, interrupt Google sync and GBrain at transaction boundaries and prove cursor/spool recovery without duplicate effects.
- [ ] Fill each quota to soft/hard limits and prove graceful degradation, notifications, protected evidence retention, and recovery after cleanup.
- [ ] Reboot host and simulate container failure, network loss, invalid secret, corrupt optional store, and corrupt required store; verify health classification and restart limits.
- [ ] Perform encrypted backup, destroy the VM data disk, restore to a new VM, rotate credentials, and prove Goal state, approvals, cursors, memory, artifacts, and audit integrity.
- [ ] Administer from an approved Tailscale device and prove LAN/unapproved/public access fails.
- [ ] Audit Git, filesystem modes, service environment/argv, journal, audit, backup output, and model transcripts for canary secrets.
- [ ] Record measured boot time, restart recovery time, backup duration/size, restore RTO, data RPO, disk headroom, and all command transcripts in a release evidence bundle.

## 13. DeepSeek batches

1. Tasks 1–3: filesystem, systemd, health.
2. Tasks 4–6: containers, secrets, backup/restore.
3. Tasks 7–9: quotas, logs, private access.
4. Tasks 10–12: upgrade and full disaster drill.

Guardrails:

```text
Do not turn internal workers into separate systemd services.
Do not expose a public daemon, health, MCP, or GBrain port.
Do not place secrets in Git, unit text, argv, logs, or model context.
Do not raw-copy live SQLite databases without a consistent snapshot method.
Do not delete active Goal evidence to recover disk space.
Do not claim backup success without a clean-host restore drill.
Stop after each batch with host-level command evidence.
```
