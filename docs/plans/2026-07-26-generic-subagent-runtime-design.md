# Generic Subagent Runtime Selection Design

**Date:** 2026-07-26
**Status:** Approved design, pending implementation plan

## 1. Purpose

Aletheon's subagent API must describe the work a child agent needs to perform,
not require the caller to know which concrete runtime happens to provide that
work. Pi, native Cognit, extension runtimes, and future Codex or Claude adapters
must enter through one runtime-neutral selection and lifecycle contract.

The normal interface is:

1. a caller selects a role/profile and describes required capabilities;
2. a broker resolves those requirements against registered runtime manifests;
3. Executive remains the sole admission and lifecycle authority;
4. an explicit runtime selector remains available only as an advanced or
   diagnostic override.

This replaces the current public contract in which `agent_spawn` requires a
concrete runtime ID and contains Pi-specific task-format guidance.

## 2. Current Code Boundary

The repository already contains part of the required substrate:

- `crates/runtime/src/manifest.rs` defines runtime capabilities, interaction
  modes, workspace modes, tool governance, and `RuntimeManifest`.
- `crates/runtime/src/selector.rs` defines automatic, alias, and
  capability-based selectors.
- `crates/executive/src/application/agent_control/execution.rs` stores launchers
  and manifests in `AgentRuntimeRegistry` and can resolve a selector.
- `crates/corpus/src/tools/tools/agent_control.rs` bypasses that substrate:
  `agent_spawn` requires a raw `runtime` string and currently documents
  `pi-rpc` and `pi-coder` explicitly.
- The canonical `AgentSpawnRequest` records a selected `RuntimeId`, so selection
  must happen before the durable run is admitted and persisted.

The change therefore extends and connects existing boundaries rather than
creating a second runtime framework.

## 3. Considered Approaches

### 3.1 Capability-first broker with profile defaults — selected

The caller supplies a profile, task, constraints, and optional capability
requirements. A broker expands the profile defaults, resolves a manifest, and
produces the existing concrete spawn request.

Benefits:

- callers are independent of Pi, Codex, Claude, and other provider names;
- profiles remain convenient without becoming hard-coded runtime aliases;
- explicit runtime selection is retained for diagnostics;
- existing Executive lifecycle and persistence remain authoritative.

Cost:

- manifests need enough metadata to make selection safe and deterministic;
- the selection decision needs its own tests and evidence.

### 3.2 Profile-to-runtime routing

Each profile directly names one concrete runtime. This is simpler, but a
profile becomes deployment-specific and cannot safely fall back when a runtime
is unavailable or lacks a requested capability.

### 3.3 Generic lifecycle with mandatory runtime IDs

All adapters implement the same launch, wait, send, cancel, and list operations,
but the caller still chooses the runtime. This standardizes execution without
solving the caller coupling or hard-coded tool guidance.

## 4. Architecture

```text
agent_spawn
    |
    v
SubagentSpawnIntent
    |  task, profile, capabilities, workspace/tool policy, budget
    |  optional runtime override
    v
SubagentBroker
    |-- expand profile requirements
    |-- inspect registered RuntimeManifest values
    |-- reject incompatible candidates
    |-- rank compatible candidates deterministically
    |-- emit RuntimeSelectionDecision
    v
AgentSpawnRequest with concrete RuntimeId
    |
    v
AgentControlService
    |-- admission
    |-- durable run/session ownership
    |-- launch, wait, send, cancel, recovery
    v
AgentRuntimeLauncher adapter
    |-- native-cognit
    |-- pi-rpc / pi-coder
    |-- registered extension runtimes
    `-- future Codex, Claude, or local runtimes
```

The broker selects; it does not launch. `AgentControlService` continues to own
authorization, admission, persistence, cancellation, recovery, and result
settlement.

## 5. Public Spawn Intent

The tool-facing request uses provider-neutral fields:

```json
{
  "profile": "researcher",
  "task": "Analyze the repository architecture and its three main risks.",
  "required_capabilities": ["code_read", "code_search"],
  "context": {},
  "tools": [],
  "budget": {
    "max_input_tokens": 120000,
    "max_output_tokens": 12000,
    "max_tool_calls": 80,
    "max_elapsed_ms": 900000,
    "max_depth": 2
  }
}
```

`runtime` becomes optional. When present, it is interpreted as an explicit
selector/alias and must still satisfy every effective requirement. It never
bypasses capability, policy, workspace, or budget checks.

Compatibility rules:

- existing requests with `runtime` remain valid;
- new requests may omit it;
- unknown fields remain rejected;
- task text is runtime-neutral at this boundary;
- runtime-specific task serialization belongs in the selected adapter;
- the generic tool schema and description contain no concrete runtime IDs.

## 6. Profiles and Effective Requirements

A profile describes role intent and safe defaults, not a concrete provider.
The initial generic roles are:

| Profile role | Default requirements | Default workspace intent |
|---|---|---|
| `researcher` | code read, code search, diagnostics | read-only shared workspace |
| `planner` | code read, code search | read-only shared workspace |
| `coder` | code read, code search, code edit, shell, test | isolated worktree |
| `reviewer` | code read, code search, diagnostics, test | read-only unless a fix is explicitly requested |
| `general` | no implicit code mutation capability | read-only |

Repository-defined profiles may add or narrow requirements. They must not
silently remove a capability explicitly required by the caller. The effective
requirement set is the union of profile requirements and caller requirements.

Profile names are not permanently hard-coded into runtime selection. Profile
loading remains configuration-driven; the table above defines behavioral
semantics and bootstrap defaults.

## 7. Runtime Manifest Contract

`RuntimeManifest` remains the source of selectable runtime facts. It is extended
only with metadata required for safe selection:

- supported task input encoding;
- supported workspace intents, including explicit read-only support;
- supported interaction modes;
- declared runtime capabilities;
- tool-governance level;
- optional configured priority;
- optional context/token constraints when they are known;
- availability/health supplied as live registry evidence rather than persisted
  as a static manifest claim.

Provider and model names are informational metadata, not selection branches.
The broker must not contain `if runtime == "pi-rpc"` or equivalent logic.

All runtime registrations exposed to subagent selection must publish a
manifest. A launcher without a manifest can still exist for compatibility, but
it is not eligible for automatic selection.

## 8. Deterministic Selection

Selection is fail-closed:

1. expand the selected profile into default requirements;
2. merge explicit capabilities and workspace/interaction constraints;
3. enumerate manifested runtimes;
4. if a runtime override is present, restrict candidates to that ID or alias;
5. reject candidates missing any required capability;
6. reject candidates incompatible with workspace, interaction, tool governance,
   or known budget/context limits;
7. reject unavailable candidates using current registry health evidence;
8. order remaining candidates by configured priority, then stable runtime ID;
9. select the first candidate;
10. return a structured not-found error when no candidate remains.

Selection must not issue a model request, retry a provider, or probe candidates
by launching them. This prevents one `agent_spawn` call from multiplying
provider requests.

## 9. Selection Evidence and Runtime Catalog

Every successful decision records:

- requested profile;
- caller requirements;
- expanded effective requirements;
- whether an override was used;
- selected runtime ID and manifest revision;
- deterministic selection reason.

Every failure reports candidate rejection reasons without exposing credentials
or private provider configuration.

The runtime registry exposes a bounded catalog projection for diagnostics and
future UI use. Model prompts should not need the full catalog during normal
automatic selection. The catalog is evidence, not a second configuration
source.

## 10. Lifecycle and Isolation

Selection does not change the canonical lifecycle:

```text
queued -> running -> waiting -> running -> succeeded
                    |           |
                    |           `-> failed/cancelled/interrupted
                    `------------> failed/cancelled/interrupted
```

Each child run retains an independent:

- agent and session identity;
- provider conversation;
- output/event stream;
- cancellation token;
- workspace lease and authorization scope;
- tool-call, inference-round, provider-retry, token, cost, and elapsed-time
  accounting.

Parent sessions observe child output only through the agent control contract.
One TUI or parent session must never receive another session's unaddressed
child output.

## 11. Errors

The generic layer distinguishes:

- invalid intent: malformed or contradictory requirements;
- profile not found;
- no compatible runtime;
- explicit override not found;
- explicit override incompatible with effective requirements;
- admission or workspace-policy rejection;
- runtime launch failure;
- provider unavailable, rejected request, or rate limit;
- timeout, cancellation, and terminal-state conflicts.

Selection errors occur before a durable child launch. Runtime and provider
errors occur after selection and retain the selected runtime ID in evidence.
Automatic selection never converts a provider failure into an unreported
attempt on another provider. Runtime failover requires a separate, explicit
retry policy so usage and side effects remain auditable.

## 12. Delivery Scope

The first implementation delivers:

1. provider-neutral spawn intent types;
2. profile requirement expansion;
3. a deterministic broker over `AgentRuntimeRegistry` manifests;
4. an optional backward-compatible runtime override;
5. removal of Pi-specific text from the generic `agent_spawn` contract;
6. manifest-backed registration for selectable runtimes currently shipped by
   the daemon;
7. runtime catalog and selection evidence sufficient for debugging;
8. deterministic contract, service, and tool-schema tests;
9. installed-runtime acceptance using the official binary, socket, and a real
   TUI request.

The first implementation does not:

- implement a new Codex or Claude process adapter;
- introduce model-driven routing;
- retry a task across providers automatically;
- replace Executive lifecycle ownership;
- force Reasoner, Planner, or Critic stages;
- create another tool-search system;
- redesign fragment/world-state storage.

After this layer lands, Codex, Claude, or another runtime can be added by
implementing `AgentRuntimeLauncher`, publishing a truthful manifest, and
registering it. No `agent_spawn` schema change should be necessary.

## 13. Verification and Acceptance

Deterministic verification must prove:

- profile and explicit requirements merge correctly;
- selection is stable regardless of manifest insertion order;
- missing capabilities and incompatible workspace modes fail closed;
- an explicit override cannot bypass requirements;
- an unmanifested runtime is excluded from automatic selection;
- at least two distinct manifested runtime implementations can satisfy the
  same generic request in tests;
- the generic tool schema contains no Pi, Codex, or Claude runtime ID;
- existing explicit-runtime requests remain compatible;
- one spawn creates one child launch and does not probe providers;
- selection evidence and rejection reasons are bounded and deterministic;
- session output and accounting remain isolated.

Repository builds and tests use `bash scripts/cargo-agent.sh`.

Because this change affects tools, profiles, daemon bootstrap, and client
behavior, final acceptance requires:

1. `sudo bash scripts/aletheon.sh deploy`;
2. identical SHA-256 digests for `target/release/aletheon`,
   `/usr/bin/aletheon`, and both running daemon executables;
3. stable systemd restart counters;
4. three consecutive real-TUI generic subagent runs through `/usr/bin/aletheon`
   and the official user socket;
5. rendered frames, durable session evidence, and daemon logs with no
   `provider_unavailable`, `provider_rejected_request`, or other inference
   error;
6. separate reporting of model inference rounds, provider retries, tool calls,
   token usage, and elapsed time.

## 14. Migration Sequence

1. Add generic intent, profile requirement, and selection-decision types.
2. Extend manifests with only the metadata needed by the broker.
3. Add deterministic selection to the Executive registry.
4. translate generic intent into the existing durable `AgentSpawnRequest`.
5. change `agent_spawn` schema and descriptions while preserving explicit
   runtime compatibility.
6. migrate shipped selectable runtime registrations to manifested
   registration.
7. add catalog/evidence projection and focused tests.
8. perform installed-runtime acceptance.

At every stage, the existing explicit runtime path remains usable. Removal of
that compatibility path requires a separate deprecation decision.
