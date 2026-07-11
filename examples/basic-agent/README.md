# Basic Agent Example

Minimal example showing how to build an agent using the `aletheon-runtime`
crate. This is the recommended starting point for new Aletheon integrations.

## What It Does

1. Loads configuration from `config.toml`
2. Creates an `AletheonExecutive` instance
3. Processes a single user input through the ReAct loop
4. Prints the agent's response

## Prerequisites

- Rust toolchain (stable, edition 2021)
- Aletheon workspace built (`cargo build` from workspace root)

## Run

```bash
# From workspace root
cargo run -p basic-agent-example

# Or from this directory
cargo run
```

## Configuration

Edit `config.toml` to change the model, provider, iteration limits, and
memory backend. See `config/default.toml` in the workspace root for the
full reference.

## Code Overview

The `main.rs` file demonstrates:

- Constructing a `ExecutiveConfig`
- Creating an `AletheonExecutive`
- Implementing the three callback traits (`review_fn`, `think_fn`, `execute_fn`)
- Calling `runtime.process()` with a user prompt
