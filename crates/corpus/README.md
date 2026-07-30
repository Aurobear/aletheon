# corpus

Governed capability execution for tools, skills, hooks, MCP extensions,
sandboxing, and optional desktop drivers.

## Current layout

```text
src/
  catalog/      runtime extension discovery
  core/         execution-body composition
  service/      governed invocation service
  tools/        built-in tools and MCP adapters
  security/     approval, policy, runner, and sandbox enforcement
  skill/        skill loading and routing
  hook/         lifecycle hook registry
  drivers/      optional input/display/a11y/OCR adapters
  extension/    extension contracts and activation
```

Host filesystem, process, PTY, service, and sandbox contracts are owned by the
`platform` crate. Android platform support is not implemented here. The former
empty `drivers/io` and `drivers/proc` placeholders have been removed.

## Validation

```bash
bash scripts/cargo-agent.sh test -p corpus
```
