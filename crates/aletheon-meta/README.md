# aletheon-meta

MetaRuntime — the self-modification engine. Like Linux kernel's module subsystem
(`modprobe`/`insmod`) handles loading/unloading kernel modules, MetaRuntime handles
loading/unloading/upgrading the agent's own runtime.

## Architecture

```
core/
  traits.rs     — DefaultMetaRuntime implementing MetaRuntimeOps
  types.rs      — Genome type re-exports + EvaluatorSpec, MorphogenesisCandidate, GenomePatch

bridge/
  (empty — adapter types as implementation matures)

impl/
  genome/
    loader.rs   — GenomeLoader: YAML file ↔ Genome struct
  meta_runtime/
    self_reader.rs     — reads agent's own genome and runtime state
    spec_editor.rs     — modifies genome specifications
    runtime_builder.rs — constructs new runtime from genome
    sandbox_runner.rs  — tests candidate runtimes in isolation
    evaluator.rs       — scores candidates after sandbox testing
    rollback.rs        — reverts to previous runtime version
    migration.rs       — transitions from old runtime to new candidate
    lineage.rs         — tracks history of runtime versions
  morphogenesis/
    pipeline.rs        — MorphogenesisPipeline: the self-evolution flow
    mutation_intent.rs — generates mutation intents from reflection
    candidate.rs       — generates candidate runtimes from genome mutations
```

## Genome YAML Format

The genome is the agent's self-description. Not code itself, but the rules
that generate code and runtime. Components:

- **Topology** — subsystem graph (name, type, version, dependencies)
- **Identity** — name, description, self-model
- **Boundary** — rules (condition → action → priority)
- **Care** — priorities (topic → weight)
- **Memory** — backends and compaction strategy
- **Mutation** — allowed targets, sandbox/approval requirements
- **Lifecycle** — auto-compact, health check interval, max idle time
- **Evaluator** — metrics with weights (defined in this crate, not ABI)

## Morphogenesis Pipeline

```
reflect → mutate spec → generate candidate → sandbox test → evaluate → migrate → become
```

1. **Reflect** — agent analyzes recent experience and reflection
2. **Mutate spec** — MutationIntentGenerator produces genome patches
3. **Generate candidate** — CandidateGenerator applies patches to genome
4. **Sandbox test** — SandboxRunner tests candidate in isolation
5. **Evaluate** — Evaluator scores candidate (strengths/weaknesses/recommendation)
6. **Migrate** — MigrationManager transitions to new runtime (requires SelfField approval)
7. **Become** — new runtime takes over, lineage is recorded

## Key Constraints

- All mutations require SelfField approval before migration
- Sandbox testing is mandatory for any genome change
- Rollback must always be available (previous version preserved)
- Lineage is append-only (history cannot be rewritten)
