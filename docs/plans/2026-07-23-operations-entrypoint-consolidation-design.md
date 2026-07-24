# Operations Entrypoint Consolidation Design

## Status

Approved.

## Problem

The repository exposes many executable shell scripts directly below `scripts/`
and several top-level shell test entrypoints. Operators must know which scripts
are public, which are internal implementation details, and which combinations
form a complete build, deployment, or acceptance workflow.

This creates three risks:

1. callers bypass the canonical deployment and installed-runtime gates;
2. internal script names become accidental public APIs;
3. CI, systemd installation, documentation, and local workflows drift onto
   different command paths.

## Goals

1. Reduce the supported operator surface to three explicit entrypoints:

   ```text
   setup.sh
   scripts/aletheon.sh
   scripts/cargo-agent.sh
   ```

2. Keep `setup.sh` at the repository root as the first-install/bootstrap
   interface.
3. Make `scripts/aletheon.sh` the single entrypoint for build, install, deploy,
   maintenance, verification, acceptance, and test workflows.
4. Move internal executable implementations out of the `scripts/` root.
5. Provide one test command namespace while preserving narrow suite execution.
6. Atomically migrate repository, CI, installer, systemd, and documentation
   references so removed paths cannot silently remain in use.

## Non-goals

- Combining all shell implementation into one large file.
- Replacing `scripts/cargo-agent.sh`, whose build-cache and lock contract
  remains independent.
- Moving or renaming `setup.sh`.
- Changing the behavior of backup, restore, upgrade, cleanup, verification, or
  acceptance operations beyond their invocation paths.
- Hiding narrow tests from CI; the unified test namespace dispatches to focused
  suites rather than forcing every job to run everything.

## Public Command Surface

### Bootstrap

`setup.sh` remains at the repository root and retains its supported bootstrap
options. It may call internal commands through `scripts/aletheon.sh`, but callers
do not need to know internal implementation paths.

### Rust build infrastructure

`scripts/cargo-agent.sh` remains a directly callable infrastructure command.
Repository agents and CI continue to invoke Cargo only through this wrapper.

### Operations

`scripts/aletheon.sh` exposes:

```text
build
install
deploy
configure
status
health
restart
logs
closure
backup
restore
upgrade
cleanup runtime
cleanup cargo
secrets init
secrets audit
database check
verify
verify systemd
verify network
verify compose
verify migration
verify multi-user
acceptance architecture
acceptance release
test unit
test operations
test deployment
test architecture
test all
help
```

Bare `verify` remains the strict installed-runtime deployment gate. Specialized
verification commands do not replace it.

## Internal Layout

Sourced dispatcher modules remain under:

```text
scripts/lib/aletheon/
```

Standalone executable implementations move to:

```text
scripts/libexec/aletheon/
├── healthcheck.sh
├── pi-scheduled-task.sh
├── secret-audit.sh
├── secret-init.sh
├── sqlite-check.sh
├── architecture-check.sh
├── backup.sh
├── cleanup.sh
├── cleanup-cargo-target.sh
├── install-systemd.sh
├── release-acceptance.sh
├── restore.sh
├── upgrade.sh
└── verify/
    ├── compose.sh
    ├── migration-matrix.sh
    ├── multi-user-runtime.sh
    ├── network-exposure.sh
    └── systemd.sh
```

The repository location communicates that these files are implementation
details. Production installation continues to expose reviewed helpers beneath
`/usr/libexec/aletheon/`; repository layout changes do not require production
unit files to reference the source checkout.

## Dispatcher Design

Focused sourced modules implement command groups:

```text
scripts/lib/aletheon/maintenance.sh
scripts/lib/aletheon/security.sh
scripts/lib/aletheon/acceptance.sh
scripts/lib/aletheon/test.sh
```

Each module:

- validates its own subcommand and argument count;
- invokes an exact internal executable path;
- uses existing common logging/error helpers;
- preserves subprocess exit status;
- does not duplicate the low-level implementation.

`scripts/aletheon.sh help` documents public commands only. Internal executable
paths are omitted from normal operator help.

## Test Layout and Dispatch

Shell test implementations move into categorized suites:

```text
tests/suites/operations/
tests/suites/deployment/
tests/suites/architecture/
tests/suites/production/
```

Non-shell fixtures and domain-specific directories remain where their owning
test harness expects them unless moving them provides a clear command-surface
benefit.

The dispatcher maps:

```text
test operations   -> operations command/static/runtime-gate suites
test deployment   -> systemd, upgrade, installed-host, and failure suites
test architecture -> architecture boundary and path inventory suites
test unit         -> bounded repository unit-test command
test all          -> ordered fail-fast composition of the above
```

CI may invoke either `scripts/aletheon.sh test <suite>` or a focused suite file
when isolation is required. New operator documentation uses the unified command.

## Migration and Compatibility Policy

The migration is atomic inside the repository:

1. move internal scripts;
2. update every repository reference;
3. update installer copy sources while retaining stable installed destinations;
4. update test paths and CI workflows;
5. add a static inventory gate that rejects references to removed root paths;
6. delete the old `scripts/*.sh` implementations in the same change.

No repository compatibility wrappers remain for removed internal paths. This
prevents the root directory from remaining crowded indefinitely. Release notes
and deployment documentation provide the old-to-new command mapping for external
callers.

## Safety Constraints

- `setup.sh`, `scripts/aletheon.sh`, and `scripts/cargo-agent.sh` cannot be moved.
- `scripts/aletheon.sh deploy` must retain the strict installed-runtime
  acceptance gate.
- Backup, restore, and upgrade keep their existing privilege, checksum,
  ownership, receipt, and fail-closed contracts.
- Secret commands must not print secret values or embed credentials in argv,
  logs, tests, or documentation.
- Tests must use temporary roots and command fixtures; they cannot mutate the
  active Aletheon user state.
- Production `/usr/libexec/aletheon` paths remain stable unless an explicit
  deployment migration is separately approved.

## Verification

The migration must prove:

1. only the three approved public shell entrypoints remain at the root/public
   levels;
2. all moved scripts pass `bash -n`;
3. every public command dispatches to the expected internal implementation;
4. subprocess failures propagate through the dispatcher;
5. installer assets remain byte-identical to their source implementations;
6. no tracked file references removed repository paths;
7. operations, deployment, architecture, production-static, and strict
   installed-runtime gate tests pass;
8. `setup.sh` behavior and help remain available at its original path.

## Acceptance Criteria

- `setup.sh` remains in the repository root.
- `scripts/` exposes only `aletheon.sh`, `cargo-agent.sh`, and implementation
  directories.
- All other former top-level scripts live under
  `scripts/libexec/aletheon/`.
- Public operations are available as `scripts/aletheon.sh` commands.
- Shell tests are categorized and available through
  `scripts/aletheon.sh test`.
- CI, docs, installer, and systemd source references use the new layout.
- A static gate fails if a removed top-level script path is reintroduced.
- Strict installed-runtime deployment verification remains mandatory.
