# corpus

Body runtime for the Aletheon agent — the "body" that executes actions.

## Overview

The `corpus` crate contains all body/execution functionality:

- **Core execution** — Body runtime and sandboxed environments
- **Drivers** — Hardware drivers and platform adapters
- **Tools** — Tool implementations, hooks, skills, MCP client
- **Security** — Security pipeline and sandbox execution

## Architecture

```
corpus/src/
├── core/           — Core execution body (BodyRuntime, conversions)
├── bridge/         — Bridge interface
├── testing/        — Mock sandbox for testing
├── drivers/        — Hardware drivers and platform adapters
│   ├── driver/     — Driver trait and types
│   └── platform/   — Platform-specific implementations (Linux, Android)
├── tools/          — Tool implementations
│   ├── hooks/      — Lifecycle hooks
│   ├── skills/     — Skill definitions
│   └── mcp/        — MCP client
└── security/       — Security pipeline
    ├── pipeline/   — Security evaluation pipeline
    └── sandbox/    — Sandboxed execution
```

## Key Types

### Core
- `BodyRuntime` — Main body execution runtime
- `Sandbox` — Sandboxed execution environment

### Drivers
- `Driver` — Hardware driver trait
- `InputDriver` — Input device driver
- `DisplayDriver` — Display driver

### Tools
- `Tool` — Tool trait for defining tools
- `ToolResult` — Tool execution result
- `HookEngine` — Lifecycle hook system
- `SkillRegistry` — Skill registration

### Security
- `SecurityPipeline` — Security evaluation pipeline
- `SandboxExecutor` — Sandboxed code execution

## Usage

```rust
use corpus::tools::{Tool, ToolResult};
use corpus::security::SecurityPipeline;
use corpus::drivers::Driver;
```

## Dependencies

- `base` — Core traits and types
