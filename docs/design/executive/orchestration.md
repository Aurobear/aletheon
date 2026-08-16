# Multi-Agent Orchestration

The retired Executive orchestration tree no longer exists. Runtime now owns the
evidence-driven orchestration state machine and iteration budget:

- `crates/runtime/src/orchestration.rs`
- `crates/runtime/src/iteration_budget.rs`
- `crates/aletheon/src/wiring/application/agent_control/mod.rs`

The runtime controller selects stages from typed risk and evidence gaps; agent
spawn, wait, cancellation, settlement, and recovery remain on the canonical
AgentSupervisor path. Historical Selector/Handoff/DiGraph module claims are not
current production contracts.
