# Metacog

Metacog is Aletheon's domain-neutral, evidence-backed subsystem for
self-observation and governed self-evolution. Coding is an outer adapter, not a
core dependency.

## Closed loop

```text
experience -> evidence -> evaluation -> problem ledger -> reflection
                                                        |
                                                        v
measured outcome <- experiment <- governed evolution <- proposal
```

Observation and modification are separate authorities. Metacog can continuously
record and recommend, but a proposal cannot approve itself or bypass approval,
sandbox evaluation, migration lineage, or rollback.

## Feature-owned architecture

```text
src/
  experience/    normalized assessable units and ingestion
  evidence/      integrity validation and append-only evidence
  evaluation/    versioned rubrics, fixed-point scoring, hard gates
  problem/       fingerprints, lifecycle events, replay projection
  reflection/    deterministic patterns and causal hypotheses
  improvement/   proposal governance, persistence, promotion bridge
  evolution/     candidates, experiments, lineage, migration, rollback
  genome/        genome model and file loading
  governance/    mutation authority and runtime facade
  adapters/      true outer-boundary translations
```

Stable cross-crate experience, evidence, and evaluation contracts live in
Fabric. Metacog does not depend on a particular capability domain, LLM provider,
database, Cargo, Git, ROS, or external agent product.

## Persistence

Evidence, problems, proposals, experiments, and causal lineage use versioned,
append-only records. Current state is rebuilt by deterministic replay.
Corrections append events rather than rewriting history. See
[`docs/deployment/metacog-problem-ledger.md`](../../docs/deployment/metacog-problem-ledger.md)
for ownership, recovery, backup, quarantine, retention, and redaction
procedures.

## Scoring and governance

- Scores use fixed-point values and only applicable dimensions participate in
  the weighted total.
- Unknown dimensions remain unknown; missing evidence is not converted to a
  zero or an automatic failure.
- Reports carry evidence coverage, confidence, rubric/evaluator versions, and
  evidence references.
- Immutable safety and policy gates override numeric scores.
- Reflection produces hypotheses and proposals, never a mutation directly.
- Promotion requires an accepted, unexpired, reversible proposal with evidence,
  validation, rollback, and external approval requirements.
- A deployed candidate is not considered successful until an experiment
  compares it with a baseline and records the measured outcome.

## Validation

Run Cargo only through the repository wrapper:

```bash
bash scripts/cargo-agent.sh test -p fabric --test metacognition_contract
bash scripts/cargo-agent.sh test -p metacog
bash scripts/cargo-agent.sh test -p executive --test coding_metacog_adapter
bash scripts/cargo-agent.sh test -p executive --test coding_metacog_rubric
bash scripts/cargo-agent.sh test -p executive --test coding_metacog_e2e
bash scripts/aletheon.sh acceptance architecture
```
