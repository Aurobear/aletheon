# SelfField Architecture

> Current conceptual architecture, verified against `crates/dasein/src/lib.rs`
> and `crates/dasein/src/core/` on 2026-07-30.

## Ownership

SelfField is Dasein's policy and read-model facade. Versioned lived-state
mutation is owned by `DaseinModule::transition`; legacy policy layers are not a
second evolution authority.

```text
intent / lived event
       |
       v
  Dasein policy facade
  identity · boundary · care · narrative · conflict · attention
       |
       +----> verdict / read projection
       |
       v
  versioned Dasein reducer
  temporality · persistence · continuity · self model
```

## Conceptual layers

| Layer | Current role |
|-------|--------------|
| Identity | Maintains the agent's declared identity and mutation history |
| Boundary | Evaluates constitutional and path/action boundaries |
| Care | Scores concerns that influence policy decisions |
| Narrative | Records continuity-oriented decision context |
| Conflict | Resolves competing intents and safety/capability tension |
| Attention | Tracks focus and priority decay |
| Continuity | Preserves lineage through the lived-state reducer and persistence |
| Mutation | Tracks governed mutation intent and approval |

These layers do not replace the authoritative enforcement owners:

- Kernel owns admission, budgets, leases, operations, and supervision.
- Corpus owns tool policy, approval, sandboxing, and guarded execution.
- Fabric owns shared risk, audit, and loop-detection contracts/infrastructure.
- Executive owns composition, turn/goal/agent orchestration, and settlement.
- Metacog owns evidence-backed candidate evaluation and evolution governance.

## Current modules

```text
crates/dasein/src/
  core/       identity, boundary, care, narrative, conflict, attention,
              continuity, mutation, store, and policy facade
  dasein/     reducer, temporality, persistence, self model, and event bridge
  bridge/     policy and loop-detector integration
  impl/       perception and mutation adapters only
```

The former standalone `ResourceGovernor`, `EmergencyKillswitch`,
`IntegrityMonitor`, guardian, watchdog, and safe-mode implementations are not
current Dasein capabilities. Installed integrity and resilience come from
runtime provenance, health, admission, cancellation, supervision, and sandbox
boundaries.

## Related documents

- [Perception](perception.md)
- [First principle](first-principle.md)
- [Writable root](writable-root.md)
- [Corpus security](../corpus/security.md)
- [Metacog](../metacog/README.md)
