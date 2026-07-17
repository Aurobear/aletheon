# Production migration and installed-daemon scenarios

V02 is a fail-closed release gate. It does not use a mock daemon, temporary
in-process coordinator, fake provider result, or a development-host service.
The staged-host scripts refuse to run unless virtualization detection confirms
a systemd-booted disposable VM/container.

Implementation of the gate is not completion evidence. Clean install, live
workflow, injected failure, and rollback evidence remain unproven until a real
disposable host run produces a passing operator receipt. In particular,
`ALETHEON_PRODUCTION_FAILURE_DRIVER` is a mandatory externally supplied real
host driver; this repository does not replace it with a fake implementation.

```
V01 acceptance
      |
migration matrix -- installed release/systemd -- real monitor workflows
      |                       |                         |
      +---------------- failure/recovery ---------------+
                              |
                 matching data+binary rollback
                              |
                    signed operator receipt
```

## Lanes

### Static lane

Run `scripts/verify-migration-matrix.sh`, shell syntax checks, monitor pytest,
Python bytecode compilation, and both typed systemd checks against a real
release binary:

```bash
scripts/verify-systemd.sh --core-unit config/aletheon-core.service \
  --binary target/release/aletheon
scripts/verify-systemd.sh --user-units \
  config/aletheon.user.service config/aletheon.user.socket \
  --binary target/release/aletheon
```

The first check covers the machine inference core; the second checks the
per-user socket-activated runtime and its 0600 endpoint. These checks are useful
before a disposable host exists, but are not release approval.

### Credential-free disposable-host lane

Inside a freshly booted, virtualization-detectable systemd VM/container, set
`ALETHEON_DISPOSABLE_HOST=1`, supply distinct `ALETHEON_BASELINE_BINARY` and
`ALETHEON_RELEASE_BINARY` artifacts, and run
`tests/production/install_upgrade_restart.sh`. The script installs the actual
binary and checked-in units, verifies readiness, controlled restart, ownership,
modes, AF_UNIX exposure, journal output, SQLite integrity, forward upgrade, and
matching data+binary rollback. External integrations stay disabled by the
production config. This lane proves installation assets without personal
credentials; it does not claim that Gmail passed.

### Live workflow and release lane

Configure an isolated Gmail test account and set
`ALETHEON_PRODUCTION_GMAIL_ACCOUNT`. Run from `tools/aletheon-monitor`:

```sh
python3 -m pytest -q tests
python3 -m src.__main__ scenario --suite production --source-root ../..
```

The four scenarios validate Git/workspace boundaries, bounded Gmail summary
evidence, SubAgent lifecycle records, and TUI reconnect/final-answer durability.
Missing credentials produce `BLOCKED` and a nonzero exit; raw mailbox data is
not emitted into the report.

The failure lane additionally requires a real executable
`ALETHEON_PRODUCTION_FAILURE_DRIVER`. Every invocation is terminated after
`ALETHEON_FAILURE_DRIVER_TIMEOUT_SECS` (default 120 seconds, maximum 600).
The matrix exports the selected user/UID, user and machine unit/socket/state
roots, peer-user state root, candidate SHA-256 and canonical provenance JSON as
`ALETHEON_FAILURE_*`; the driver must act on those exact installed boundaries.

The driver protocol is:

```text
prepare PHASE BEFORE.json
verify PHASE BEFORE.json AFTER.json
inject FAILURE RECEIPT.json
recover FAILURE RECEIPT.json
backup-matching BACKUP_ROOT BACKUP.json
restore-matching BACKUP.json RESTORE.json
compare-v01 RESTORE.json V01.json COMPARISON.json
```

`PHASE` is one of `event_append`, `memory_lease`, `gbrain_remote_success` or
`agent_runtime_completion`; `FAILURE` is one of `queue_full`, `disk_full`,
`corrupt_supplement`, `provider_timeout` or `tui_disconnect`. Each receipt must
copy the exported provenance and candidate digest, contain the matrix-verifiable
machine/target-user/peer-user state hashes, set `cross_scope_leak` to `false`,
and contain an empty `ignored_cases` array. Prepare/verify and inject/recover
pairs must preserve one non-empty `acknowledged_work.id` and advance its state
from acknowledged/observed to settled without silent loss.

`backup-matching` must create the requested non-symlink backup root and return a
non-empty `backup_id`, `status: "complete"`, and
`matching_binary_and_state: true`. `restore-matching` must restore that same
backup ID with the candidate binary and return `status: "restored"`, matching
state hashes and fresh runtime provenance. Only then may `compare-v01` compare
the restored installed state with the V01 projections. A mock driver or a JSON
receipt that is not bound to these identities cannot establish release evidence.

## Aggregate release gate

Run `just release-acceptance` only inside the disposable host. Required inputs:

- the real release executable (`ALETHEON_RELEASE_BINARY`);
- the previous released executable (`ALETHEON_BASELINE_BINARY`), whose digest
  must differ from the candidate;
- a passing V01 JSON report (`ALETHEON_V01_ACCEPTANCE_REPORT` when non-default);
- the production failure driver;
- isolated production credentials for live-account cases;
- `ALETHEON_RELEASE_OPERATOR` for the final receipt;
- `just`, `jq`, `sqlite3`, systemd tooling, tmux, Python pytest, and Cargo.

The gate first invokes `just acceptance`; this prerequisite cannot be bypassed.
It then validates the emitted event/projection checksums and projection
inventory, positive Agent/mailbox reopen counts, recovered memory lease, zero
unexpected external calls, every functional indicator, strictly reducing
workspace/recurrence/Dasein ablations, and the architecture-recipe marker.
The failure lane accepts that architecture marker only with the aggregate
gate's guest-local V01 recipe receipt, whose report checksum must match the
validated report immediately produced by `just acceptance`; a standalone
report cannot assert that the recipe ran.
It requires a clean `target/release-acceptance` directory and zero blocked or
ignored cases. Default time bounds are 30 seconds for readiness, 120 seconds for
ordinary TUI workflows, and 180 seconds for SubAgent/reconnect workflows.
Installed-host and failure lanes write under a unique guest-local
`/var/tmp/aletheon-release-acceptance.*` root. On success, failure, or BLOCKED,
the gate copies those receipts and logs into `target/release-acceptance/guest`
and records the original guest path without using the source checkout as the
rollback staging area.

### Aggregate receipt provenance

The installed-host drill proves a matching baseline rollback and then reapplies
the candidate through the production upgrade path before returning. The
aggregate gate rejects the run unless the monitor preflight
`binary_sha256`, the installed-host receipt `candidate_sha256`, and both live
processes named by the failure receipt resolve to that same candidate digest.

The final operator receipt embeds a release-case inventory derived from V01,
the installed-host lane, all four named monitor scenarios, and the failure
lane. `ignored_release_cases` is calculated from that validated inventory; it
is not a constant. Any failed, blocked, skipped, or ignored inventory entry
prevents receipt creation.

`lane_evidence` records the SHA-256 digest and bundle-relative path for the V01
report and recipe receipt, migration receipt, installed-host receipt, candidate
activation receipt, monitor report, failure receipt, architecture receipt,
dependency tree, and case inventory. The operator receipt also records the
digest of this lane-evidence manifest so copied guest artifacts can be checked
without trusting path names or a friendly `PASS` string alone.

## Rollback decision

- No data change and explicit matrix compatibility: a verified binary rollback
  may be considered, but must retain release evidence.
- Any data change or unknown compatibility: stop service; preserve upgraded
  roots; restore the matching pre-upgrade data/config to empty roots; install
  the matching saved binary; preflight; start; verify readiness, SQLite
  integrity, and V01 projection/state checksums.
- Never start an old binary against migrated data. Never run mixed binaries
  against shared durable roots.
