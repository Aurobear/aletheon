# V02 Production Migration and Real Scenarios Implementation Plan

> **Status:** Open — no passing disposable-host operator receipt has been retained

**Goal:** Prove installation, migration, restart, rollback, bounded failure and real user workflows against the installed daemon before release.

**Architecture:** A disposable installed host runs the actual release binary, systemd assets and data migrations; monitor scenarios validate durable effects and failure recovery before release approval.

**Tech Stack:** Bash, systemd verification, Python pytest, aletheon-monitor, SQLite integrity tooling

**Source requirements:** `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md:1162-1170`; V02 also closes the production behavior required by the preceding source-plan acceptance clauses.

**Prerequisite:** V01.

**Operational validation assumption:** The project-workspace, Gmail, SubAgent and TUI cases below come from the user's requested real-scenario validation, not from an additional claim in the four source plans. They validate V02 deployment behavior but do not expand source-plan feature scope.

## Current-code anchors

- Native installation is implemented by `scripts/install-systemd.sh:20-62` and verified by `scripts/verify-systemd.sh:36-97`.
- Upgrade preserves the previous binary and runs forward migrations in `scripts/upgrade-aletheon.sh:42-100`.
- Backup/restore and rollback constraints are documented at `docs/deployment/backup-restore.md:3-10` and `docs/deployment/upgrade-rollback.md:3-8`.
- The real debug harness drives installed TUI scenarios and host-side verification at `tools/aletheon-monitor/src/scenarios.py:1-24`.

## Invariants and non-goals

- Live personal accounts are not mandatory for CI; production credentials are opt-in fixtures.
- Binary-only rollback is forbidden after incompatible data migration.
- Physical crate splitting remains an evidence-based ADR decision.

## Key contracts

```toml
[[transition]]
component = "event_spine"
from = 1
to = 2
backup_required = true
rollback = "restore_matching_data_and_binary"
```

## Task 1: Define release manifest and migration compatibility matrix

**Create:** `config/release/migration-matrix.toml`
**Create:** `scripts/verify-migration-matrix.sh`
**Modify:** `docs/deployment/upgrade-rollback.md`

- [ ] List source/target schema versions for event, Session, memory, Agent, Agora, Dasein and config state.
- [ ] Define forward migration, mixed-version prohibition, backup requirement and rollback method per transition.
- [ ] Verify every migration has a pre-migration fixture, reopen test and post-migration integrity query.
- [ ] Reject binary-only rollback after incompatible data migration.

Run: `scripts/verify-migration-matrix.sh`

## Task 2: Build an isolated installed-daemon test host

**Create:** `tests/production/lib/installed_host.sh`
**Create:** `tests/production/install_upgrade_restart.sh`
**Modify:** `scripts/verify-systemd.sh`

- [ ] Stage real release binaries, config, credentials, systemd units and writable roots in a disposable VM/container namespace.
- [ ] Run install, readiness, controlled restart, upgrade and matching data+binary rollback.
- [ ] Verify ownership/modes, AF_UNIX exposure, health, journald output and graceful shutdown.
- [ ] Preserve all receipts, logs and database integrity output as artifacts.

Run: `tests/production/install_upgrade_restart.sh`

## Task 3: Add real user workflow scenarios

**Create:** `tools/aletheon-monitor/scenarios/project_workspace.py`
**Create:** `tools/aletheon-monitor/scenarios/gmail_analysis.py`
**Create:** `tools/aletheon-monitor/scenarios/subagent_research.py`
**Create:** `tools/aletheon-monitor/scenarios/reconnect_resume.py`

- [ ] Project scenario starts in a known working directory, reads/writes only approved workspace paths and reports Git state accurately.
- [ ] Gmail scenario uses a configured test account, handles unauthorized/degraded states and returns a stable bounded summary without dumping raw payloads.
- [ ] SubAgent scenario exercises spawn, progress, mailbox, cancellation, result promotion and daemon restart.
- [ ] TUI scenario exercises long tool output, scrolling, reconnect and final-answer persistence.
- [ ] Validate returned evidence, session records and logs instead of matching only friendly prose.

Run: `cd tools/aletheon-monitor && python -m pytest -q tests && python -m src.__main__ scenario --suite production`

## Task 4: Exercise bounded failure and recovery

**Create:** `tests/production/failure_matrix.sh`

- [ ] Kill daemon after event append, memory lease, remote GBrain success and Agent runtime completion.
- [ ] Inject full queue, disk-full boundary, corrupt supplemental response, provider timeout and lost TUI connection.
- [ ] Verify authoritative local state, idempotent recovery, explicit degraded health and no silent result disappearance.
- [ ] Restore from a matching backup and compare V01 projection/state checksums.

Run: `tests/production/failure_matrix.sh`

## Task 5: Add release gate and rollback drill

**Create:** `scripts/release-acceptance.sh`
**Create:** `docs/testing/production-scenarios.md`
**Modify:** `justfile`

- [ ] Run V01, migration matrix, installed-daemon, real scenarios, failure matrix and backup/restore verification.
- [ ] Require clean artifacts, explicit operator receipt and zero ignored release cases.
- [ ] Document exact rollback decision points and time bounds.

Run: `just release-acceptance`

## Task 6: Record the optional physical-split decision

**Create:** `docs/decisions/adr-app-protocol-and-extension-sdk-split.md`

- [ ] Measure actual dependency edges and external ABI needs after all gates pass.
- [ ] Split Fabric protocol/transport or an extension SDK only if it removes a verified dependency edge or stabilizes an external ABI.
- [ ] Record a justified no-split decision when evidence does not meet that threshold.

Run: `mkdir -p target/release-acceptance && cargo tree --workspace --edges normal > target/release-acceptance/dependency-tree.txt`

## Final verification and commit

Run: `scripts/architecture-check.sh && cargo test --workspace --all-targets --no-fail-fast`

Inspect the staged diff, then commit with subject `test(release): gate installed production scenarios` and a body that records the source requirement, authority/bypass problem, implemented boundaries, focused tests and deletion evidence.

## Completion evidence

- [ ] Clean install, upgrade, restart and matching rollback pass on the staged host.
- [ ] Real project, Gmail, SubAgent and TUI scenarios produce durable verifiable results.
- [ ] Failure injection leaves no silent loss, cross-scope leak or unrecoverable acknowledged work.
- [ ] Physical crate splitting is evidence-driven rather than mandatory.
