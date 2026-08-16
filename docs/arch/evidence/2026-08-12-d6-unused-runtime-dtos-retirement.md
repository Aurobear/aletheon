# D6 unused runtime DTO retirement

- `crates/fabric/src/include/runtime.rs` exposed four scheduler/agent DTOs only
  through Fabric's compatibility re-exports; repository-wide symbol discovery
  found no production or test caller.
- D6 deleted `AgentInfo`, `AgentStatus`, `ScheduledTask`, and `ScheduleKind`
  instead of moving an unused schema into Runtime.
- The Fabric public inventory and boundary census removed the same four rows;
  no replacement compatibility path was introduced.

## Caller-zero receipt

```text
rg -n '\b(AgentInfo|AgentStatus|ScheduledTask|ScheduleKind)\b|include::runtime' \
  crates --glob '*.rs' --glob '!crates/fabric/src/include/runtime.rs'

Before deletion: only crates/fabric/src/lib.rs compatibility re-exports.
After deletion: zero matches.
```
