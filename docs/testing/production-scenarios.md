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
`ALETHEON_DISPOSABLE_HOST=1`, supply `ALETHEON_RELEASE_BINARY`, and run
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

The failure lane additionally requires a real
`ALETHEON_PRODUCTION_FAILURE_DRIVER`. The driver establishes and verifies the
actual daemon boundary for event append, memory lease, remote GBrain success,
and Agent runtime completion. It must also provide bounded disposable-scope
queue-full, disk-full, corrupt-response, provider-timeout, and disconnect
receipts. A mock driver cannot establish release evidence.

## Aggregate release gate

Run `just release-acceptance` only inside the disposable host. Required inputs:

- the real release executable (`ALETHEON_RELEASE_BINARY`);
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

## Rollback decision

- No data change and explicit matrix compatibility: a verified binary rollback
  may be considered, but must retain release evidence.
- Any data change or unknown compatibility: stop service; preserve upgraded
  roots; restore the matching pre-upgrade data/config to empty roots; install
  the matching saved binary; preflight; start; verify readiness, SQLite
  integrity, and V01 projection/state checksums.
- Never start an old binary against migrated data. Never run mixed binaries
  against shared durable roots.
