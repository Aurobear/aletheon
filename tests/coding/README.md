# Engineering coding benchmark

This directory contains a black-box engineering benchmark for the real
`aletheon exec` client:

```text
strict task catalog
  -> isolated fixture + typed setup
  -> real client terminal snapshot
  -> independent acceptance + Git/resource checks
  -> sealed per-task receipt
  -> sealed deterministic suite report
```

- `fixtures/` contains twenty independent miniature repositories; they are not
  workspace crates.
- `tasks/` contains strict version-1 TOML contracts for twenty versioned scenario
  categories.
- `acceptance/` contains overlays copied only after client execution.
- `harness/run.py` executes one task and always attempts to emit a version-2
  receipt.
- `harness/replay.py` checks receipt integrity, terminal/evidence correlation,
  failure classification, and false-success rejection.
- `harness/suite.py` runs tasks sequentially and writes a sealed version-1 suite
  report.

Natural-language model output is bounded diagnostic evidence only. The host
verdict comes from the authoritative stop, operation correlation, Git scope,
independent commands, resource cleanup, and the declared expected terminal.

## Deterministic checks

Run the secret-free contract suite with:

```sh
bash tests/coding/static_test.sh
```

It validates all twenty task documents plus runner, receipt, aggregation, and
workflow contracts. Fixture Cargo commands are always routed through
`scripts/cargo-agent.sh`; do not invoke Cargo directly.

## Diagnostic execution

Build through the shared lock and run one task:

```sh
bash scripts/cargo-agent.sh build -p aletheon
python3 tests/coding/harness/run.py \
  tests/coding/tasks/rust_bugfix.toml \
  --receipt tests/coding/receipts/current/rust_bugfix.json
```

Run the catalog sequentially:

```sh
python3 tests/coding/harness/suite.py \
  --catalog tests/coding/tasks \
  --receipts tests/coding/receipts/current \
  --report tests/coding/receipts/current/suite.json
```

Without `ALETHEON_BIN`, the harness resolves the same shared target directory as
`scripts/cargo-agent.sh`, including `CARGO_TARGET_DIR`,
`ALETHEON_CARGO_CACHE_ROOT`, and `XDG_CACHE_HOME` overrides. The client needs a
reachable configured inference core. Sandbox mode defaults
to `auto` and may be set with `ALETHEON_CODING_SANDBOX=auto|require|forbid`.
Each command and client process runs in a separate process group, captured text
is bounded, and the receipt retains SHA-256 digests of the complete streams.

Suite exit codes are stable:

| Exit | Meaning |
|---:|---|
| `0` | Every task matched its expected terminal and all independent gates passed |
| `1` | At least one correlated task failed execution, policy, or verification |
| `2` | Catalog, binary, core, transport, runner, or receipt infrastructure failed |

`blocked` and `budget_exhausted` are valid benchmark outcomes only for tasks
that explicitly expect them and prove their side-effect invariants. They are not
counted as completed engineering tasks in the normal success numerator.

## Manual real-model workflow

The GitHub Actions workflow **Real Coding Evaluation** is manual-only. It uses
the owner-provided `LEJU_API_KEY`, pins the `leju` provider and
`deepseek/deepseek-v4-pro`, starts one inference core, then runs the twenty tasks
sequentially through `harness/suite.py`.

GitHub-hosted runners use `ALETHEON_CODING_SANDBOX=forbid` because they may
expose bubblewrap while denying its namespace operations. This exception is
limited to disposable fixture copies. The workflow always uploads:

```text
coding-artifacts/
├── core.log
├── receipts/
│   └── <task>.json
└── suite.json
```

The credential-bearing core configuration and temporary HOME are outside that
artifact directory and are removed on exit.

## Acceptance boundary

A run using `target/debug/aletheon`, a temporary HOME, an alternative socket, or
a directly started core is **diagnostic evidence only**. It is not installed
runtime acceptance.

Changes that affect client behavior, tools, configuration, persistence, IPC, or
daemon bootstrap are complete only after all repository deployment checks pass:

1. `sudo bash scripts/aletheon.sh deploy` succeeds;
2. `target/release/aletheon`, `/usr/bin/aletheon`, and both running daemon
   executables have identical SHA-256 digests;
3. systemd restart counters remain stable; and
4. `/usr/bin/aletheon` completes a real request through the official user
   socket.
