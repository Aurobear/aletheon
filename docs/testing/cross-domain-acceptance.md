# Cross-domain acceptance suite

The V01 suite proves causal behavior across the real Executive conscious-workspace coordinator,
Kernel process tree, Dasein adapter, Agora SQLite broadcast store, Agent SQLite repository, and
the five event-spine projections. External memory content is a local adversarial fixture and all
network, provider, credential, and child-process execution paths are absent from the harness.

```text
fixed Clock + IDs + local provider fixtures
                 |
                 v
Kernel Process tree -> Executive ConsciousWorkspaceRegistry
                              |
                  +-----------+-----------+
                  |                       |
             Agora SQLite          Dasein integration
                  |                       |
                  +---- causal trace -----+
                              |
          Session / debug / memory / Agent / metrics reducers
```

## Run

Run the complete gate with `just acceptance`. On hosts without `just`, execute the recipe commands
from `justfile` directly. The two Rust test binaries are serial test commands at the gate boundary;
each individual test owns a fresh temporary data root.

The gate rejects ignored acceptance tests, known unbounded-wait constructs, and fixture projection
inventory/version drift. It writes `target/acceptance/acceptance.json` and a concise Markdown copy.

## Evidence and interpretation

- `cross_domain_acceptance` covers deterministic replay, recurrence, governed action outcome,
  repository reopen, adversarial recall authority, sibling process/worktree isolation, promotion
  lineage, and bounded failure receipts.
- `functional_indicators` measures the indicators defined by the source requirement and compares
  workspace, recurrence, and Dasein ablations against the same trace schema.
- `ConsciousCoreTrace` contains causal IDs, salience, policy versions, acknowledgements, integration
  versions, action permits/outcomes, and memory receipt authority. It cannot carry hidden reasoning.
- The checked-in fixture is semantic input and expected inventory, not a golden copy of random IDs.

These measurements support a functional architecture claim only. They do not establish phenomenal
consciousness, and model prose or self-report is never acceptance evidence.
