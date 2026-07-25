# Agent Pattern Absorption Benchmark

This benchmark is the admission gate for selectively adopting agent patterns.
It measures the installed Aletheon runtime; development binaries and alternate
sockets are invalid evidence.

```text
/usr/bin/aletheon
        |
        v
$XDG_RUNTIME_DIR/aletheon/aletheon.sock
        |
        v
raw output + turn metrics -> versioned receipt
```

Run the receipt contract test:

```bash
bash tests/production/agent_pattern_absorption.sh
```

Record the immutable three-run baseline:

```bash
bash scripts/libexec/aletheon/agent-pattern-benchmark.sh \
  --variant baseline --runs 3
```

Artifacts are written under `target/agent-pattern-absorption/<variant>/`.
Every receipt records installed-binary provenance, the official socket,
success, input/output/cache-hit tokens, latency, and tool calls. The
50-message case uses a unique explicit session so it does not rely on the
client's default-session selection.

Run one case while developing the harness:

```bash
bash scripts/libexec/aletheon/agent-pattern-benchmark.sh \
  --variant smoke --runs 1 --case simple_read_only
```

An increment remains disabled unless its three-run median improves the
case-specific success or operability signal and stays within that track's
declared token and latency thresholds.
