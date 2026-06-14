# Self-Evolution Demo

Demonstrates Aletheon's self-evolution loop: the agent receives a task, plans
a solution, executes it, reflects on the outcome, and stores learned rules in
memory so that future runs improve automatically.

## Scenario

The agent is asked to format a set of log files into a summary report.
On the first run it produces a raw concatenation. After reflection the agent
stores a rule ("group by severity, deduplicate"), and on the second run the
output is a well-structured report -- demonstrating closed-loop learning.

## Prerequisites

- Rust toolchain (stable)
- Aletheon built at workspace root (`cargo build`)

## Quick Start

```bash
# 1. Prepare the environment (creates sample logs, config)
bash setup.sh

# 2. Run the demo
bash run-demo.sh
```

## Expected Output

```
=== Run 1: Initial execution ===
[INFO] Processing 5 log files ...
[INFO] Output written to output/run1-report.txt
[REFLECT] Noted: raw output is too verbose, should group by severity.
[LEARN] Stored rule: group-by-severity

=== Run 2: After learning ===
[INFO] Processing 5 log files ...
[INFO] Output written to output/run2-report.txt
[REFLECT] Output quality improved. Rule applied: group-by-severity
[LEARN] No new rules -- output meets quality bar.
```

## Files

| File | Purpose |
|------|---------|
| `README.md` | This file |
| `setup.sh` | Creates sample data and config |
| `run-demo.sh` | Executes the two-pass demo loop |
| `config.toml` | Agent configuration for the demo |
