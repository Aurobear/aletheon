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
