# Multi-Agent Orchestration

The retired Executive orchestration tree no longer exists. The evidence-driven
orchestration controller (role workflow state machine) now lives in Agora as the
single controller, while Runtime retains the iteration budget:

- `crates/agora/src/cognitive_role_workflow/mod.rs`
- `crates/runtime/src/iteration_budget.rs`
- `crates/aletheon/src/composition/agent_control/mod.rs`

The runtime controller selects stages from typed risk and evidence gaps; agent
spawn, wait, cancellation, settlement, and recovery remain on the canonical
AgentSupervisor path. Historical Selector/Handoff/DiGraph module claims are not
current production contracts.
