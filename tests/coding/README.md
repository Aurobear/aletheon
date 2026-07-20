# Coding production harness

`fixtures/` contains independent miniature repositories; they are not workspace crates.
`tasks/` defines prompts, timeouts, path scopes, and independent acceptance commands.
`harness/run.py` launches the real `aletheon exec` binary in an isolated copied workspace.
`receipts/` stores bounded replay records. `harness/replay.py` validates correlation,
integrity, diff evidence, acceptance commands, verification, and terminal status.

Build without bypassing the shared Cargo lock, then run a task:

```sh
bash scripts/cargo-agent.sh build -p aletheon
ALETHEON_BIN="$PWD/target/debug/aletheon" \
  python3 tests/coding/harness/run.py tests/coding/tasks/rust_bugfix.toml
```

A provider/core suitable for real work must be configured. Missing inference, timeout,
failed acceptance, out-of-scope edits, and false success all produce a failed receipt.

## Manual real-model evaluation

The GitHub Actions workflow **Real Coding Evaluation** is intentionally available only
through manual dispatch. It uses the repository secret `LEJU_API_KEY` with the pinned
`leju` provider and `deepseek/deepseek-v4-pro` model, starts one inference core, and runs
`rust_bugfix`, `rust_multifile`, and `rust_diagnosis` sequentially.

The manual workflow sets `ALETHEON_CODING_SANDBOX=forbid` because GitHub-hosted
runners can expose a bubblewrap binary while denying the namespace operations it
requires. This exception applies only to disposable fixture copies inside the ephemeral
runner; local harness runs remain `auto`, and independent acceptance still decides the
result.

Every run uploads one receipt per fixture and `core.log` as the
`coding-e2e-<run-id>` artifact, including when the evaluation fails. The generated
credential-bearing configuration and temporary HOME are never uploaded and are removed
when the evaluation step exits.

Failures before a correlated operation receipt exists (missing secret, core readiness,
or provider/core transport) are infrastructure failures. A correlated operation that
finishes but violates fixture acceptance is an agent verification failure. Both fail the
manual workflow, but the preserved receipts and log keep the distinction auditable.
