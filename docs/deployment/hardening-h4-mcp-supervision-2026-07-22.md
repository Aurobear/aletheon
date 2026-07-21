# H4 MCP task supervision evidence — 2026-07-22

## Requirement anchors

- MCP health/reconnect tasks must register a task name, termination reason, cancellation signal,
  and bounded shutdown (`docs/plans/2026-07-21-production-readiness-hardening.md:177-187`).
- Panic or abnormal exit must become degraded health with an explicit restart/stop decision
  (`docs/plans/2026-07-21-production-readiness-hardening.md:184-186`).
- Fault injection must demonstrate degradation and recovery/stop behavior; shutdown must leave no
  newly admitted tasks and must have a timeout; normal reconnect must not regress
  (`docs/plans/2026-07-21-production-readiness-hardening.md:189-192`).
- This batch is MCP-first and does not mechanically wrap reasoning logger, perception, or synchronous
  Mnemosyne work (`docs/plans/2026-07-21-production-readiness-hardening.md:186-187`).

## Implemented lifecycle

```text
McpConnectionManager
        |
        +-- register server: connecting
        +-- spawn named task through McpTaskSupervisor
        |      +-- notifications (normal channel close = complete; panic = degraded)
        |      +-- health loop   (unexpected return/panic = degraded)
        |      `-- initial reconnect (success = complete; panic = degraded)
        |
        +-- ping failure --> reconnecting --> replacement --> connected
        +-- health RPC --> core readiness + external_dependencies.mcp
        `-- shutdown --> stop admission --> cancel --> bounded join --> abort timeout
```

- The supervisor owns task registration, cancellation, bounded health history, panic capture,
  explicit normal-exit policy, sanitized termination reasons, server health, and bounded shutdown
  (`crates/corpus/src/tools/mcp/supervisor.rs:14-290`).
- Health, notification, and initial-reconnect tasks all enter that supervisor and select on its
  cancellation token (`crates/corpus/src/tools/mcp/client.rs:703-717,858-926,954-986,1133-1209`).
- One-shot reconnect completion is explicitly `Complete`; long-running health exit is
  `DegradeOnExit`. Notification channel closure is a normal connection-generation turnover, while
  a panic still degrades the server (`crates/corpus/src/tools/mcp/supervisor.rs:41-47,157-237`).
- The daemon retains `McpManager`, projects MCP status separately from core readiness, and invokes a
  five-second bounded shutdown before the rest of the runtime shuts down
  (`crates/executive/src/impl/daemon/handler/mod.rs:19-35,199-212`,
  `crates/executive/src/impl/daemon/handler/rpc/rpc_health.rs:99-149`).
- The architecture gate prevents production MCP client code from reintroducing an unsupervised
  `tokio::spawn` (`scripts/architecture-check.sh:107-112`).

## Fault-injection and regression evidence

Commands run through the repository wrapper:

```bash
bash scripts/cargo-agent.sh test -p corpus tools::mcp --lib
bash scripts/cargo-agent.sh test -p executive external_mcp_degradation_is_distinct_from_core_readiness --lib
bash scripts/cargo-agent.sh fmt --all -- --check
bash scripts/architecture-check.sh
git diff --check
```

Observed deterministic coverage:

- injected task panic is caught and appears as `failed/panic` plus server `degraded`;
- the chosen policy stops the failed task rather than silently restarting potentially corrupted
  task state;
- initial connection failure enters `reconnecting`, later connects, emits the registry signal, and
  reaches `connected` without marking expected one-shot completion as failure;
- an established server returning failures becomes `reconnecting`, then returns to `connected`
  after the mock server recovers;
- cooperative shutdown completes tasks, closes task admission, and marks servers `stopped`;
- a deliberately non-cooperative task is aborted after the configured timeout and recorded as
  `aborted/shutdown_timeout`;
- all 85 MCP unit tests pass, including existing authentication, tool, resource, notification, and
  reconnect contracts;
- daemon external dependency degradation is represented independently of core liveness/readiness.

## Operational behavior and rollback

- Health reason values are fixed bounded categories (`ping_failed`, `initial_connect_failed`,
  `initial_connect_failed_reconnect_disabled`, `panic`, `unexpected_exit`, `shutdown_timeout`);
  transport errors and credentials are not copied into health output.
- MCP degradation does not make core liveness false. Operators can distinguish a ready core from a
  degraded optional external dependency through `external_dependencies.status` and the MCP
  snapshot.
- Rollback is the independent H4 commit. It changes no persistent schema.
