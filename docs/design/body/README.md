# Body Crate (`corpus`)

> The body crate provides the physical interface layer — tools, sandbox, MCP, platform adaptation, security policy, drivers, UI, and ACIX.

**Crate:** `corpus`
**Source:** `crates/corpus/`
**Last updated:** 2026-06-14

---

## Crate Structure

```
crates/corpus/
├── core/           — Core types and traits
├── bridge/         — FFI and cross-crate bridges
├── impl/           — Implementation modules
│   ├── tools/      — Tool system (Tool trait, registry, built-in tools, output defense)
│   ├── sandbox/    — Sandbox execution (bubblewrap, process, noop backends)
│   ├── mcp/        — MCP client (stdio, HTTP, SSE transports, OAuth)
│   ├── security/   — Security policy (PolicyEngine, RiskClassifier, AuditLogger, RollbackEngine)
│   ├── platform/   — Platform adaptation (Linux, Android, boot, IPC, awareness)
│   ├── driver/     — Hardware drivers (display, input, OCR, a11y, process, I/O)
│   ├── ui/         — Terminal UI (chat, commands, markdown, skills, status)
│   ├── acix/       — Agent-Computer Interface (grounding, experience, task management)
│   └── mod.rs      — Module entry point
└── testing/        — Test utilities and fixtures
```

## Documents

| Document | Scope |
|----------|-------|
| [tools.md](tools.md) | Tool system — trait, registry, built-in tools, output defense, ToolExposure, parallel execution |
| [sandbox.md](sandbox.md) | Sandbox execution — bubblewrap/process/noop backends, environment detection |
| [mcp.md](mcp.md) | MCP integration — client, transports (stdio/HTTP/SSE), OAuth, tool wrapping |
| [security.md](security.md) | Security policy — PolicyEngine, RiskClassifier, AuditLogger, RollbackEngine, multi-agent permissions |
| [platform.md](platform.md) | Platform adaptation — PlatformAdapter, boot integration, agent awareness, kernel IPC, multi-device |
| [perception.md](perception.md) | Perception layer — event sources, aggregation, backpressure (source: perception-layer.md) |
| [fuse.md](fuse.md) | FUSE virtual filesystem — mount structure, state provider, controls |
| [driver.md](driver.md) | Hardware drivers — display (X11/DRM), input (uinput), OCR, accessibility, process, I/O |
| [ui.md](ui.md) | Terminal UI — chat, commands, computer view, markdown rendering, skills |
| [acix.md](acix.md) | Agent-Computer Interface — ACI protocol, grounding, experience memory, task management |

## Internal Pattern

Each `impl/` module follows the core/bridge/impl/testing pattern:

- **core/** — shared types and trait definitions
- **bridge/** — cross-crate integration points
- **impl/** — concrete implementations
- **testing/** — test utilities and mocks
