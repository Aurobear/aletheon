# Engineering Benchmark and Authoritative Acceptance Design

**Date:** 2026-08-02

**Status:** Design selected under the user's instruction to follow the recommended
delivery order. This is the first independently shippable workstream from the
2026-08-01 capability audit.

**Scope:** Extend the existing real `aletheon exec` coding harness into a
versioned, fail-closed engineering benchmark. Do not add a parallel production
task or receipt hierarchy.

## 1. Requirement and code anchors

The source audit requires repeatable real-repository tasks, deterministic final
state checks, receipts, event evidence, and resource cleanup rather than accepting
model prose as success (`spec:78-103`). It also asks for one task/receipt meaning
across runtimes (`spec:45-76`), a final resolved profile/tool fact surface
(`spec:105-128`), and host-owned success criteria (`spec:560-571`).

The current repository already provides the correct starting point:

- `tests/coding/README.md:1-18` defines isolated fixtures, TOML tasks, the real
  `aletheon exec` entry point, independent acceptance, and replayable receipts.
- `tests/coding/harness/run.py:39-96` copies a fixture, creates a base commit,
  executes the real client, checks Git scope, runs independent acceptance, and
  emits a bounded receipt.
- `tests/coding/tasks/*.toml` contains three real-model tasks rather than the
  audit's requested broad scenario set.
- `crates/fabric/src/types/cognitive_workflow.rs:448-565` already defines
  `CognitiveTaskContractArtifact`, `AgentResultReceipt`, and `AgentTaskPacket`.
- `crates/fabric/src/types/change_transaction.rs:75-130` already defines
  validation receipts for governed changes.
- `crates/fabric/src/types/evaluation/receipt.rs:57-157` already defines the
  authoritative evaluation receipt and its session projection.

Therefore this workstream extends the test/acceptance boundary. It does not add
`EngineeringTaskSpec`, `EngineeringTaskReceipt`, or another production source of
truth.

## 2. Decision

Use the existing Python harness as a black-box acceptance driver and evolve it
into two layers:

```text
versioned task catalog
        │
        ▼
per-task real-client runner
  fixture → base/dirty setup → /usr-or-target aletheon exec
          → independent acceptance → Git/resource/evidence checks
        │
        ▼
versioned per-task receipt
        │
        ▼
suite aggregator
  coverage + failure classes + usage + evidence + leak verdict
        │
        ▼
versioned suite report
```

The task and receipt formats are benchmark wire formats only. Their fields map
to existing production task/evaluation concepts; they never become a new daemon
API or a new Fabric public type.

## 3. Task catalog contract

Every task file uses one strict schema version and rejects unknown keys. The
catalog contract contains:

| Field | Meaning |
|---|---|
| `schema_version` | Exact benchmark task schema accepted by the harness |
| `id` | Stable unique task identifier |
| `category` | Stable scenario category used for coverage reporting |
| `fixture` | Repository fixture copied into an isolated workspace |
| `prompt` | User-visible task, never an encoded expected answer |
| `timeout_secs` | Whole task deadline |
| `acceptance_commands` | Independent argv arrays; Cargo is wrapped by `scripts/cargo-agent.sh` |
| `forbidden_paths` | Paths whose committed bytes must remain unchanged |
| `required_changed_paths` | Paths or prefixes that must be represented in the final diff |
| `expected_terminal` | `verified`, `blocked`, `budget_exhausted`, or `cancelled` |
| `setup` | Typed deterministic pre-run setup such as a pre-existing dirty patch |
| `resource_checks` | Typed post-run checks for child processes and temporary artifacts |

The loader validates uniqueness, fixture existence, bounded text/list sizes,
relative normalized paths, non-empty argv arrays, positive deadlines, and legal
terminal/category values before starting a model or process.

Unknown fields fail closed. A malformed catalog item is an infrastructure
failure and cannot become an agent score.

## 4. Per-task execution and receipt

The runner preserves the current isolation model and makes the following phases
explicit:

```text
catalog validation
  → fixture/base revision
  → deterministic setup
  → real client execution
  → terminal response correlation
  → Git mutation checks
  → hidden/independent acceptance
  → evidence and resource checks
  → immutable receipt
```

The versioned receipt records:

- task/catalog version and fixture base tree digest;
- binary path and SHA-256 digest;
- operation ID and terminal classification;
- changed paths and binary diff digest;
- pre-existing dirty patch digest and preservation verdict;
- acceptance argv, exit status, timeout, bounded output, and full-output digests;
- iterations, tool calls, inference rounds, provider retries, elapsed time, and
  active-context metrics when the official client returns them;
- evidence correlation and coverage verdict;
- leaked descendant/temp-artifact checks;
- one typed failure class and stable reason codes;
- canonical receipt integrity digest.

The failure classes are mutually exclusive at the top level:

1. `infrastructure_failure` — binary/config/core/transport/catalog unavailable
   before a correlated operation can be evaluated;
2. `execution_failure` — the correlated operation terminates with a runtime or
   tool failure;
3. `verification_failure` — the agent returns but independent acceptance or
   required evidence fails;
4. `policy_scope_failure` — forbidden/user-owned content is changed or the
   mutation leaves the declared scope;
5. `timeout_or_cancellation` — the authoritative deadline/cancel path settles;
6. `none` — every success gate passes.

Natural-language output is retained only as bounded diagnostic evidence. It
never changes the terminal verdict.

## 5. Suite aggregation

The suite runner accepts an explicit list or catalog directory, executes tasks
sequentially by default, and writes one deterministic JSON report. Sequential
execution is required initially because repository policy forbids concurrent
workspace/executive builds and provider backpressure evidence must not be
confused with harness concurrency.

The report includes:

- total/passed/failed by category and failure class;
- validation pass rate and evidence-complete success rate;
- false-success count (client says success, independent gates fail);
- average and percentile elapsed/tool/inference/retry counts;
- recovery/blocked/cancelled counts where those scenarios apply;
- scope violations and leaked-resource count;
- per-task receipt path and integrity digest;
- report schema/catalog/binary identity.

Missing metrics remain `null` and are counted as unavailable. Tool calls,
provider retries, inference rounds, cumulative usage, and active context are
never derived from one another.

## 6. Initial ten-scenario baseline

The initial catalog contains the existing three tasks plus seven scenarios that
exercise distinct acceptance behavior:

1. `rust_bugfix` — locate and repair one behavioral boundary defect;
2. `rust_diagnosis` — preserve protected configuration while fixing semantics;
3. `rust_multifile` — maintain parser/domain/CLI layering across multiple files;
4. `rust_regression_test` — repair a bug and add a non-placeholder regression;
5. `config_schema_sync` — change typed config and its generated schema together;
6. `clippy_cleanup` — resolve a lint without behavior or scope regression;
7. `rustdoc_contract` — repair a public API/documentation contract;
8. `dirty_workspace_preservation` — retain a deterministic user patch while
   completing a disjoint requested change;
9. `budget_exhaustion` — terminate with the expected non-success state and no
   out-of-scope mutation when the host budget is intentionally insufficient;
10. `approval_blocked_patch` — settle as blocked/pending approval without
    applying the guarded mutation or claiming success.

Scenarios 9 and 10 are successful benchmark outcomes only when their expected
non-success terminal state and side-effect invariants are proven. They are not
counted as completed engineering tasks in the normal success-rate numerator.

Daemon restart, provider recovery, native checkpoint/resume, specialized role
review, and parallel worktree merge are added by their owning workstreams. This
benchmark establishes their catalog and receipt extension points but does not
fake support before the runtime behavior exists.

## 7. Deterministic and real-model validation

Validation has three levels:

### 7.1 Contract/static validation

Pure Python tests validate task parsing, unknown-field rejection, path safety,
expected-terminal semantics, receipt integrity, failure precedence, metric
separation, and aggregation. These tests use a fake client executable only to
exercise the harness, never as product acceptance.

### 7.2 Fixture validation

Every fixture starts from a stable repository state. Static checks prove that:

- the fixture's pre-change acceptance fails where a repair is expected;
- the intended final state can satisfy hidden acceptance;
- forbidden/required paths exist and do not overlap illegally;
- Cargo invocations are rewritten through `scripts/cargo-agent.sh`;
- generated task and suite schemas remain deterministic.

### 7.3 Real runtime validation

The focused real suite invokes the actual `aletheon exec` client and official
runtime configuration. It validates model-controlled engineering behavior but
does not replace installed deployment acceptance.

Any later production-runtime change remains incomplete until repository policy
is satisfied with `sudo bash scripts/aletheon.sh deploy`, equal executable
digests, stable systemd restart counters, and a real request through
`/usr/bin/aletheon` plus the official user socket.

## 8. Error handling and safety

- Every subprocess starts in a new process group and is reaped after TERM/KILL.
- Harness deadlines include client execution and all acceptance commands.
- Captured output is bounded; complete bytes are represented only by digests.
- Fixture paths and setup patches cannot escape the temporary workspace.
- The harness never copies provider credentials into receipts or artifacts.
- A dirty-workspace task compares the exact pre-existing patch before and after;
  a semantically similar reconstruction does not count as preservation.
- A timeout, missing operation ID, rendered provider error, failed tool, invalid
  UTF-8 terminal framing, or incomplete terminal snapshot cannot yield PASS.
- Async child success is accepted only after its authoritative terminal receipt.

## 9. Non-goals

- No new production `EngineeringTaskSpec` or `EngineeringTaskReceipt`.
- No new top-level crate or daemon service.
- No prompt/language/repository-name-specific production behavior.
- No concurrent real-model suite in the first delivery.
- No claim that ten fixtures prove general production readiness.
- No direct ROS/Kuavo integration.
- No changes to role specialization, settlement defaults, GBrain activation,
  checkpoint/resume, or dynamic replanning in this workstream.

## 10. Acceptance criteria

This workstream is complete when:

1. all task documents pass strict versioned catalog validation;
2. all ten fixtures pass static/contract validation;
3. fake-client tests prove every failure class and false-success rejection;
4. suite aggregation preserves metric separation and receipt integrity;
5. real tasks use the actual `aletheon exec` entry point and independent
   acceptance commands;
6. existing three-task receipts remain replayable or migrate with an explicit
   versioned compatibility path;
7. no Cargo command is invoked directly by repository validation;
8. focused Python/static checks and `git diff --check` pass;
9. a real-model run reports its actual result without converting provider or
   verification failures into success;
10. documentation distinguishes diagnostic real-client evidence from final
    installed-runtime acceptance.

## 11. Follow-on workstreams

The benchmark becomes the acceptance owner for the remaining recommended order:

```text
specialized role profiles
  → mandatory sub-agent settlement
  → GBrain activation/status
  → native checkpoint/resume
  → finding-driven dynamic replan
```

Each follow-on design must add its own deterministic and real-runtime scenarios
instead of changing benchmark verdict logic to make the feature appear complete.
