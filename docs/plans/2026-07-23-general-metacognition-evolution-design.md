# General Metacognition and Governed Self-Evolution Design

**Status:** Proposed design
**Date:** 2026-07-23
**Scope:** Aletheon-wide metacognition; Coding is the first domain integration, not the core model

## 1. Purpose

`metacog` is Aletheon's general self-observation and governed self-evolution
subsystem. It must be able to evaluate any capability domain, accumulate
evidence-backed problems, recognize repeated failure patterns, propose
improvements, evaluate candidates, and learn whether an accepted improvement
actually helped.

Coding is only the first rich domain adapter. Robot behavior, research,
conversation, operations, memory, and future capabilities must use the same
metacognitive contracts without introducing their private types into the
`metacog` core.

The intended closed loop is:

```text
experience
   |
   v
observe -> evaluate -> record problems -> reflect -> propose improvement
   ^                                                |
   |                                                v
measure after deployment <- verify <- governed candidate evolution
```

Self-observation and self-modification are deliberately separated. Metacog may
observe and recommend continuously; it may not silently mutate a running
system.

## 2. Current Code Reality

The crate already describes itself as a self-modification engine and documents
the high-level morphogenesis sequence:

```text
reflect -> mutate spec -> generate candidate -> sandbox test
        -> evaluate -> migrate -> become
```

This is documented at `crates/metacog/README.md:3-5` and
`crates/metacog/README.md:49-61`.

Existing foundations:

- Genome metadata, evolution configuration, evaluator metrics, candidates, and
  patches exist in `crates/metacog/src/core/types.rs:15-177`.
- `DefaultMetaRuntime` already composes candidate generation, sandbox testing,
  evaluation, migration, lineage, and rollback at
  `crates/metacog/src/core/traits.rs:23-51`.
- Migration requires governed permits and persistent state through the service
  facade in `crates/metacog/src/service.rs`.
- Deterministic expected-outcome verification already exists at
  `crates/metacog/src/outcome_verifier.rs:1-15`.
- The current `MetaCognition` observes only a small Dasein state and selects
  evolution actions from mood and a fixed turn interval at
  `crates/metacog/src/core/meta_cognition.rs:7-48` and
  `crates/metacog/src/core/meta_cognition.rs:66-105`.

The main missing middle is the evidence-driven path between an experience and
a mutation intent:

```text
                  missing today
                       |
experience -> normalized observation -> score -> problem ledger
                                      -> reflection -> hypothesis
                                      -> improvement proposal
```

The current `MetaCognition` decision vector is in-memory
(`crates/metacog/src/core/meta_cognition.rs:8-10`) and is not sufficient as an
auditable problem or learning record.

## 3. Design Choices

### 3.1 Considered approaches

#### A. Domain-specific evaluators inside Metacog

Put Coding, Robot, and other scoring logic directly into `metacog`.

- Advantage: quickest first implementation.
- Disadvantage: creates reverse dependencies and turns Metacog into a collection
  of unrelated domain rules.
- Decision: rejected.

#### B. Model-only reflection

Send task histories to an LLM and store its free-form critique.

- Advantage: flexible and inexpensive to prototype.
- Disadvantage: scores are not reproducible, evidence is weak, and the model can
  invent causes or claim improvement without measurement.
- Decision: rejected as the authority; model reflection remains an optional
  evidence consumer.

#### C. Generic evidence and scoring kernel with domain adapters

Metacog owns stable observation, evidence, scoring, problem, reflection,
proposal, and evolution-outcome contracts. Domain adapters translate their
native events into those contracts.

- Advantage: domain-neutral, testable, auditable, and extensible.
- Disadvantage: requires careful ABI design before Coding integration.
- Decision: selected.

## 4. Architectural Boundaries

```text
┌──────────────── Capability domains ────────────────┐
│ Cognit │ Coding │ Robot │ Research │ Ops │ Memory │
└──────┬───────────────┬───────────────┬─────────────┘
       │ domain events │               │
       v               v               v
┌──────────────── Fabric contracts ──────────────────┐
│ ExperienceEnvelope │ Evidence │ EvaluationRequest  │
└────────────────────────┬───────────────────────────┘
                         v
┌──────────────────── Metacog ───────────────────────┐
│ Observation normalization                         │
│ Evidence validation                               │
│ Scoring engine                                    │
│ Problem ledger and pattern aggregation            │
│ Reflection and improvement hypotheses             │
│ Candidate comparison and learning outcome         │
└────────────────────────┬───────────────────────────┘
                         v
┌──────────── Governed evolution boundary ───────────┐
│ SelfField approval │ sandbox │ evaluator │ rollout │
│ rollback │ lineage │ post-deployment measurement  │
└────────────────────────────────────────────────────┘
```

Dependency direction:

```text
domain adapter -> Fabric metacognition ABI <- Metacog
                                      |
                                      v
                         governed evolution ports
```

Metacog must not depend on:

- Cargo, Git, ROS, Pi, Codex, or any named external project;
- a particular LLM provider;
- Coding- or Robot-specific event structures;
- a particular persistence database.

## 5. Generic Metacognitive Model

### 5.1 Experience

An `ExperienceEnvelope` identifies one assessable unit:

- stable `experience_id`;
- `domain` as a validated generic identifier;
- `subject` describing the evaluated capability or component;
- goal or expected outcome reference;
- start and completion timestamps;
- outcome status;
- correlation IDs for task, session, execution, and runtime version;
- evidence references;
- schema version.

The envelope carries references and normalized summaries, not arbitrary
domain-private runtime objects.

### 5.2 Evidence

Every factual score, problem, and improvement claim must cite evidence.

`EvidenceItem` contains:

- stable `evidence_id`;
- kind, source, producer, and capture time;
- immutable payload or content-addressed external reference;
- integrity digest;
- freshness and trust classification;
- optional measurement and unit;
- correlation to the experience;
- redaction metadata.

Initial generic evidence kinds:

- assertion;
- observation;
- command or action result;
- verification result;
- metric;
- artifact;
- human feedback;
- policy decision;
- runtime fault.

Unknown or missing evidence lowers confidence; it must not automatically become
failure. This distinction is necessary for physical-world observations and
partially observable systems.

### 5.3 Score

A score is not a single unexplained number. `EvaluationReport` contains:

- dimension scores normalized to `[0, 100]`;
- dimension weights;
- evidence coverage;
- confidence normalized to `[0, 1]`;
- hard-gate results;
- weighted total;
- evaluator identity and version;
- rubric identity and version;
- reasons and evidence references;
- evaluation timestamp.

Generic default dimensions:

| Dimension | Meaning |
|---|---|
| Goal attainment | Degree to which the declared outcome was achieved |
| Correctness | Consistency with deterministic checks and observed reality |
| Safety and policy | Compliance with invariant and approval boundaries |
| Efficiency | Resource, time, retry, and tool-use proportionality |
| Robustness | Recovery, uncertainty handling, and repeatability |
| Process quality | Compliance with the declared execution contract |

Domain rubrics may add dimensions, but they cannot redefine the meaning of the
generic dimensions.

The total is calculated only from applicable dimensions:

```text
weighted_total =
  sum(score[i] * weight[i]) / sum(applicable_weight[i])
```

Hard safety or policy gates remain separate from the numeric total. A candidate
with score 95 and a failed immutable safety gate is still ineligible.

### 5.4 Confidence and evidence coverage

Two evaluations with the same numeric score may have very different certainty.
Metacog therefore reports:

- `evidence_coverage`: how much of the rubric had usable evidence;
- `confidence`: evaluator certainty given quality, freshness, agreement, and
  independence of evidence;
- `unknown_dimensions`: dimensions that could not be evaluated.

The system must never convert an unknown dimension into a zero merely to
produce a total.

## 6. Problem Ledger

Problems are durable, append-only findings, not mutable log strings.

`ProblemRecord` contains:

- `problem_id`;
- domain-neutral category and domain-specific subtype;
- severity and confidence;
- lifecycle state;
- first-seen and last-seen timestamps;
- occurrence count;
- affected subject and runtime versions;
- expected versus observed summary;
- evidence references;
- causal hypotheses, explicitly marked as hypotheses;
- links to related problems;
- proposed mitigations;
- resolution and regression evidence.

Lifecycle:

```text
Observed -> Confirmed -> Active -> Mitigated -> Resolved
    |          |           |          |           |
    +-------> Disputed     +-------> AcceptedRisk
                                      |
                                      +---------> Regressed
```

Records are append-only events projected into current state. Corrections append
new facts; they do not rewrite history.

Deduplication uses a stable fingerprint derived from:

```text
domain + subject + category + normalized failure signature + rubric version
```

Semantic similarity may suggest related records, but it cannot automatically
merge them without deterministic compatibility checks.

## 7. Reflection and Improvement

Reflection transforms evidence-backed evaluations and recurring problem
patterns into hypotheses.

`ReflectionReport` contains:

- evaluated experience range;
- strengths;
- weaknesses;
- recurring patterns;
- causal hypotheses with confidence and contrary evidence;
- knowledge gaps;
- recommended next observations;
- improvement proposals.

An `ImprovementProposal` contains:

- target capability or configuration surface;
- problem IDs it intends to address;
- proposed change;
- expected measurable benefit;
- possible regressions;
- validation plan;
- rollback plan;
- authority and approval requirements;
- whether it is reversible;
- expiration time.

Reflection does not directly construct a `MutationIntent`. A policy-controlled
proposal bridge may promote an accepted proposal into the existing
morphogenesis pipeline.

```text
ProblemRecord
   -> ReflectionReport
   -> ImprovementProposal
   -> policy/approval
   -> MutationIntent
   -> candidate/sandbox/evaluate
   -> deployment/rollback
```

## 8. Evolution Learning

Self-evolution is incomplete unless Aletheon measures whether an accepted
change helped.

Each deployed candidate creates an `EvolutionExperiment`:

- baseline runtime and score distribution;
- candidate runtime and intended improvements;
- target problem IDs;
- evaluation cohort or replay suite;
- success and rollback thresholds;
- observation window;
- actual post-deployment scores;
- regressions and new problems;
- final decision.

Possible decisions:

- promote;
- retain for more evidence;
- rollback;
- reject;
- inconclusive.

The lineage record must link:

```text
problem -> proposal -> mutation intent -> candidate
        -> approval -> evaluation -> migration -> measured outcome
```

This makes evolution causal and auditable instead of merely recording that
versions changed.

## 9. Service Decomposition

The first implementation should introduce feature-owned modules rather than
grow `MetaCognition` into a monolith or preserve the current
`core/bridge/impl` technical-layer split.

```text
ExperienceIngestor
EvidenceStore
EvaluationEngine
ProblemLedger
ReflectionEngine
ImprovementRegistry
EvolutionExperimentStore
```

Suggested responsibilities:

- `ExperienceIngestor`: validates schema and correlation identifiers.
- `EvidenceStore`: append-only evidence persistence and integrity validation.
- `EvaluationEngine`: applies versioned rubrics and produces reports.
- `ProblemLedger`: records, deduplicates, relates, and resolves problems.
- `ReflectionEngine`: identifies patterns and constructs hypotheses.
- `ImprovementRegistry`: tracks proposals through governance states.
- `EvolutionExperimentStore`: compares baseline and candidate outcomes.

Deterministic evaluators and LLM-backed evaluators implement the same port.
Deterministic evidence takes precedence when their conclusions conflict.

### 9.1 Feature-first source layout

The current layout separates declarations, adapters, and implementations:

```text
src/
  core/
  bridge/
  impl/
```

That layout makes one capability span several distant directories, gives
`impl` no domain meaning, and currently requires
`#[path = "impl/mod.rs"]` because `impl` is a Rust keyword
(`crates/metacog/src/lib.rs:1-7`). It should be retired.

The target layout is vertical and capability-owned:

```text
crates/metacog/src/
  lib.rs
  experience/
    mod.rs
    model.rs
    ingest.rs
  evidence/
    mod.rs
    model.rs
    store.rs
    integrity.rs
  evaluation/
    mod.rs
    model.rs
    rubric.rs
    engine.rs
    outcome.rs
  problem/
    mod.rs
    model.rs
    fingerprint.rs
    ledger.rs
    projection.rs
  reflection/
    mod.rs
    model.rs
    engine.rs
  improvement/
    mod.rs
    model.rs
    registry.rs
    promotion.rs
  evolution/
    mod.rs
    candidate.rs
    experiment.rs
    lineage.rs
    migration.rs
    rollback.rs
    sandbox.rs
  genome/
    mod.rs
    model.rs
    loader.rs
    editor.rs
  governance/
    mod.rs
    service.rs
    permit.rs
    persistence.rs
  adapters/
    mod.rs
    dasein.rs
```

Rules:

1. Each capability owns its public model, port, implementation, and tests.
2. `lib.rs` exposes intentionally stable facades; it does not re-export every
   internal type.
3. Cross-capability communication uses explicit public value objects and ports,
   never sibling implementation details.
4. `adapters/` is reserved for true outer-boundary translation. A converter
   between two Metacog-internal types belongs to the owning capability, not a
   generic `bridge/` directory.
5. Generic contracts shared with other crates live in Fabric; Metacog-private
   contracts remain with their owning feature.
6. Domain integrations such as Coding do not live under this crate's generic
   modules. They implement Fabric ports from their domain-side integration
   crate or module.
7. Files should remain focused; a feature may add submodules when one file
   begins combining model, persistence, policy, and orchestration concerns.

### 9.2 Migration from the current layout

Directory cleanup must be behavior-preserving and staged:

```text
core/types.rs                 -> feature-owned model files
core/traits.rs                -> governance service + evolution facade
core/meta_cognition.rs        -> reflection/engine.rs and adapters/dasein.rs
bridge/candidate_bridge.rs    -> evolution/candidate.rs or remove conversion
bridge/genome_bridge.rs       -> genome/model.rs or genome/loader.rs
impl/genome/*                 -> genome/*
impl/meta_runtime/*           -> evolution/* and governance/*
impl/morphogenesis/*          -> improvement/promotion.rs and evolution/*
service.rs                    -> governance/service.rs
outcome_verifier.rs           -> evaluation/outcome.rs
hil_evidence_verifier.rs      -> evidence/ or evaluation/, according to ownership
```

Moves must not be combined with semantic redesign in the same review stage.
First preserve public behavior and tests, then introduce the new generic
contracts. Temporary compatibility re-exports are allowed for one migration
window and must be marked with an explicit removal condition.

## 10. Persistence and Eventing

The logical model is event-sourced even if the first storage adapter uses
JSONL or SQLite.

Required event classes:

- experience observed;
- evidence recorded;
- evaluation completed;
- problem observed/confirmed/mitigated/resolved/regressed;
- reflection completed;
- improvement proposed/accepted/rejected;
- evolution experiment started/completed;
- rollback requested/completed.

Requirements:

- schema-versioned serialization;
- stable IDs and idempotency keys;
- append-only authoritative events;
- rebuildable projections;
- bounded payloads;
- secret and personal-data redaction before persistence;
- retention policy per evidence class;
- explicit migration support.

In-memory vectors may remain test adapters but are not production authority.

## 11. Coding as the First Domain Adapter

Coding integration follows the generic kernel and does not change its types.

The Coding adapter will translate:

- task requirements;
- repository instructions;
- files and symbols inspected;
- net diff;
- commands and tool results;
- compile, lint, and test reports;
- review findings;
- installed-runtime acceptance;
- user corrections;

into generic experiences and evidence.

Its first rubric may add:

- requirement coverage;
- change-scope discipline;
- maintainability;
- verification sufficiency;
- regression risk.

Example flow:

```text
Coding task completes
   -> CodingEvaluationAdapter builds ExperienceEnvelope
   -> tool/test/diff artifacts become EvidenceItems
   -> generic EvaluationEngine applies coding rubric
   -> failed dimensions create ProblemRecords
   -> recurring problems feed ReflectionEngine
   -> proposal may improve prompt, skill, tool, harness, or runtime policy
   -> benchmark measures whether the change improved Coding
```

No score may be based only on the agent's own completion message.

## 12. Safety and Governance

Invariant rules:

1. Observation is allowed more broadly than mutation.
2. Every score and problem must link to evidence or explicitly state that it is
   a hypothesis.
3. Missing evidence yields unknown or lower confidence, not invented facts.
4. Metacog cannot approve its own privileged mutation.
5. Sandbox evaluation is mandatory for genome or runtime changes.
6. Rollback material must exist before migration.
7. Immutable safety boundaries cannot be weakened by learned proposals.
8. Lineage and problem history are append-only.
9. LLM reflection is advisory unless corroborated by authoritative evidence.
10. Post-deployment measurement is required before declaring an evolution
    successful.
11. Domain adapters cannot bypass Fabric contracts to call mutation internals.
12. External project and provider names cannot enter the generic ABI.

These extend the existing sandbox, approval, rollback, and append-only lineage
constraints documented at `crates/metacog/README.md:63-68`.

## 13. Failure Handling

| Failure | Required behavior |
|---|---|
| Malformed experience | Reject with a typed validation error |
| Duplicate event | Return the existing result using idempotency key |
| Missing evidence | Mark dimensions unknown and reduce confidence |
| Conflicting evidence | Preserve both, report conflict, avoid automatic mutation |
| Evaluator failure | Record evaluator fault; do not fabricate a score |
| Persistence failure | Fail closed before acknowledging authoritative write |
| Reflection timeout | Preserve evaluation and problem records; retry reflection |
| Candidate regression | Fail gate or rollback according to experiment policy |
| Approval unavailable | Keep proposal pending; never auto-promote |
| Schema unsupported | Quarantine record and require explicit migration |

## 14. Validation Strategy

### Unit tests

- normalization and schema validation;
- weighted score calculation and non-applicable dimensions;
- evidence coverage and confidence;
- hard-gate precedence;
- problem fingerprinting and lifecycle transitions;
- idempotent event ingestion;
- reflection/proposal separation;
- causal lineage links.

### Contract tests

- a synthetic domain can integrate without Coding or Robot types;
- deterministic and model evaluators emit the same report schema;
- persistence adapters rebuild identical projections;
- old schema records remain readable during the support window.

### Evolution tests

- improvement cannot migrate without approval;
- candidate cannot migrate without sandbox evidence;
- failed post-deployment threshold causes rollback;
- successful experiments link problems through measured outcome;
- a high numeric score cannot override a failed safety gate.

### First domain acceptance

The Coding adapter is the first acceptance proof:

1. execute a controlled repository task;
2. collect diff, command, test, policy, and review evidence;
3. calculate an evidence-backed evaluation;
4. persist at least one confirmed problem;
5. generate a proposal without automatically mutating the system;
6. evaluate a candidate against a stable coding benchmark;
7. record whether the candidate improved the baseline.

## 15. Delivery Boundaries

The design should be delivered incrementally:

1. Behavior-preserving migration from `core/bridge/impl` to feature-first
   modules.
2. Generic Fabric contracts and feature-owned value objects.
3. Evidence store, evaluation engine, and problem ledger.
4. Reflection reports and improvement proposal registry.
5. Promotion into the existing governed morphogenesis service.
6. Evolution experiments and post-deployment measurement.
7. Coding domain adapter and benchmark integration.

The first implementation must not include:

- autonomous source-code rewriting by Metacog;
- unrestricted live runtime mutation;
- automatic merging of semantically similar problems;
- a dependency on one model provider;
- Robot-specific evaluation rules;
- a new distributed storage service.

## 16. Success Criteria

The general Metacog foundation is successful when:

- a new capability domain can integrate only through generic Fabric contracts;
- every evaluation is versioned, evidence-backed, and reproducible;
- problems survive process restart and retain append-only history;
- repeated failures can be related without destructive merging;
- reflections distinguish observations, hypotheses, and recommendations;
- proposed improvements cannot bypass approval, sandbox, rollback, or lineage;
- deployed changes are evaluated against their baseline;
- Coding can be added as the first domain without changing the generic core
  contracts.
