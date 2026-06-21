# base

Core subsystem trait contracts and shared types for the Aletheon agent framework.

## Overview

The `base` crate provides the foundational abstractions that all other crates depend on. It follows Linux kernel naming conventions:

- **`include/`** — Subsystem trait contracts (analogous to kernel headers)
- **`types/`** — Shared data types (Agent, Genome, Tool, etc.)
- **`events/`** — Event system types and infrastructure
- **`ipc/`** — Inter-process communication (Transport, Envelope, Protocol)
- **`kernel/`** — Core infrastructure (Registry, Observable, Debug, Error)
- **`policy/`** — Execution policy engine
- **`dasein/`** — Phenomenological self-awareness module

## Architecture

```
include/     ← Trait contracts for body, brain, memory, runtime, self_field
types/       ← Shared data structures
events/      ← Event types + EventBus + EventLog
ipc/         ← CommunicationBus + Transport + Backends
kernel/      ← Registry + Observable + Debug + Error
policy/      ← ExecPolicy evaluation
dasein/      ← Phenomenological module
```

## Key Types

- `Subsystem` — Lifecycle trait for all subsystems
- `Transport` — Async message transport trait
- `CommunicationBus` — Unified IPC entry point
- `Envelope` — Wire-format message container
- `EventBus` — Publish/subscribe event system

## Usage

```rust
use base::include::subsystem::{Subsystem, SubsystemContext};
use base::ipc::transport::Transport;
use base::ipc::bus::communication_bus::CommunicationBus;
```

## Dependencies

None — this is the foundation crate.
