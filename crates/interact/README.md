# interact

User interaction layer for the Aletheon agent.

## Overview

Provides the user-facing interfaces: TUI (terminal UI), CLI, and ACIX (computer interaction).

## Architecture

```
ui/        — Terminal UI with ratatui
├── app/   — Application lifecycle
├── render/ — Rendering components
└── ...    — Individual UI modules

cli/       — Command-line interface
acix/      — Computer interaction (screenshot, click, type)
tools/     — ACIX tool implementations
```

## Key Types

- `App` — TUI application
- `Args` — CLI arguments
- `Aci` — Computer interaction interface
