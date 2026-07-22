# H11 Executive Composition Evidence — 2026-07-22

## Requirement

H11 requires inference, memory, integrations, agents, tools and sessions to become typed
composition units, without reading global environment state, while preserving behavior and leaving
MCP/security/settlement decomposition to evidence-driven follow-up
(`docs/plans/2026-07-21-production-readiness-hardening.md:288-299`).

## Result

```text
RequestHandler::new
  -> inference::compose(input)       -> provider
  -> memory::compose(input)          -> core / recall / facts
  -> sessions::compose(input)        -> initial / registry / default / timestamps
  -> tools::compose(input)           -> registry + transferred stores
  -> integrations::compose_google()  -> enabled | disabled | degraded
  -> agents::compose(input)          -> profiles / tool profiles / active profile
  -> existing domain wiring and DaemonComposition
```

- Inference receives only an injected `InferencePort` and model specification
  (`crates/executive/src/impl/daemon/bootstrap/inference.rs:9-22`).
- Memory receives only the injected state root and clock; recall and fact databases remain below
  that root (`crates/executive/src/impl/daemon/bootstrap/memory.rs:11-40`).
- Sessions receive data root, session ID, context window and clock, and return all related state as
  one typed result (`crates/executive/src/impl/daemon/bootstrap/sessions.rs:12-55`).
- Tools receive typed network/search/memory/clock dependencies and return the registry while
  explicitly transferring memory stores to later composition
  (`crates/executive/src/impl/daemon/bootstrap/tools.rs:11-45`).
- Optional Google construction distinguishes disabled configuration from a degraded setup error;
  degraded setup remains warning-and-disable rather than failing the core daemon
  (`crates/executive/src/impl/daemon/bootstrap/integrations.rs:13-63`).
- Agent composition receives the directory, injected inference/provider, tool definitions and typed
  configs, and returns validated profiles plus deterministic active selection
  (`crates/executive/src/impl/daemon/bootstrap/agents.rs:12-65`).
- The request bootstrap now shows these calls and resource transfers in construction order
  (`crates/executive/src/impl/daemon/bootstrap/request.rs:73-80,123-126,260-313,844-881`).

No new unit reads process environment. MCP, security, settlement and other existing state machines
were deliberately not split as part of H11.

## Independent contracts

| Unit | Covered contract |
|---|---|
| inference | injected model/provider construction |
| memory | valid state root and invalid fact-root failure |
| sessions | complete state construction and invalid typed input failure |
| tools | required memory tools registered from injected dependencies |
| integrations | disabled is healthy; setup error is degraded and non-fatal |
| agents | configured/default fallback order and missing-profile fail closed |

## Validation

| Command | Result |
|---|---|
| `bash scripts/cargo-agent.sh test -p executive --lib` | PASS, 551 tests |
| `bash scripts/cargo-agent.sh fmt --all -- --check` | PASS |
| `bash scripts/architecture-check.sh` | PASS, no additions |
| `git diff --check` | PASS |

The repository-wide strict Clippy command remains red on pre-existing
`clippy::uninlined_format_args` findings in Platform and Executive outside these composition units;
it is not reported as an H11 pass and was not expanded into unrelated cleanup.
