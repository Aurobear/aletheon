# Specialized Cognitive Role Profiles and Capability Contract Design

**Date:** 2026-08-02

**Status:** Approved for implementation planning

**Scope:** `SUB-P0-1` only. This design gives Planner, Explorer, Executor,
Tester, Reviewer, and Fixer distinct runtime profiles and makes their tool,
artifact, and workspace authority enforceable by the Host.

## 1. Requirement and code anchors

| Requirement | Current code | Alignment |
|---|---|---|
| Six specialized roles with distinct read, write, validation, and finding boundaries (`Aletheon_Engineering_Capability_Gap_Audit_2026-08-01.md:416-427`) | Bootstrap clones one `code-agent` launch profile for all six roles (`crates/executive/src/host/daemon/bootstrap/services.rs:173-202`) | No |
| Planner and Reviewer cannot write, Executor cannot forge review, and Fixer cannot leave finding scope (`Aletheon_Engineering_Capability_Gap_Audit_2026-08-01.md:470-472`) | Typed role profiles already distinguish write authority and artifact kinds (`crates/fabric/src/types/cognitive_workflow.rs:83-265,666-696`), but the native runtime reconstructs a writable current-directory policy (`crates/executive/src/adapters/runtime/native_cognit.rs:249-290,808-827`) and findings have no path scope (`crates/fabric/src/types/cognitive_workflow.rs:500-514`) | Partial |
| Do not replace code-level authority with prompts; require an end-to-end fixture (`Aletheon_Engineering_Capability_Gap_Audit_2026-08-01.md:560-569`) | Role output is Host-validated, but profile selection and filesystem authority are not yet role-specific | Partial |

There is no requirement/code disagreement about present behavior. The audit's
description of a cloned base profile matches the current bootstrap composition.

## 2. Goals

1. Ship one profile identity and prompt for each cognitive role.
2. Resolve all six profiles explicitly; never silently substitute `code-agent`.
3. Enforce the effective tool contract after universal tools and configuration
   overrides have been applied.
4. Make non-writer workspaces read-only at the capability boundary.
5. Restrict Executor writes to the admitted task scope.
6. Restrict Fixer writes to paths attached to the exact unresolved findings.
7. Preserve Host authority over artifact type and stage advancement.
8. Prove the boundaries with deterministic negative-path fixtures and installed
   runtime acceptance.

## 3. Non-goals

- No unified cross-runtime `ExecutionReceipt` (`SUB-P0-2`).
- No settlement-default change (`SUB-P0-3`).
- No checkpoint/resume or parallel worktree merge behavior.
- No `ResolvedAgentProfileSnapshot` implementation.
- No new top-level crate, service, planner, or memory authority.
- No model choice based on a natural-language task or repository name.

## 4. Architecture

```text
bundled role Markdown
       │ load + universal-tool merge + override
       ▼
effective AgentProfile ── validate exact role capability contract
       │
       ▼
RoleLaunchProfile(role, profile_id, effective tools)
       │
       ▼
AgentTaskPacket + host-only CognitiveTaskRuntimeBinding
       │                         │
       │                         └─ task/finding workspace scope
       ▼
AgentControl admission ── attenuate parent authority, then narrow roots
       │
       ▼
Native Cognit runtime ── exact Host WorkspacePolicy + tool intersection
       │
       ▼
CognitiveRoleOutput ── artifact kind, version, evidence, and scope gates
```

Prompts explain responsibilities. Markdown tool lists define requested
capabilities. Neither is authoritative on its own: the effective profile is
validated at bootstrap, AgentControl owns workspace attenuation, and the
cognitive workflow owns typed artifact acceptance.

## 5. Shipped role profiles

Add these bundled Markdown profiles:

| Profile | Explicit role tools | Host workspace authority | Required artifact |
|---|---|---|---|
| `planner-agent` | repository inspection, file/artifact read, search, code graph | read-only | `Plan` |
| `explorer-agent` | repository inspection, file/artifact read, search, code graph | read-only | `Investigation` or `Evidence` |
| `executor-agent` | read/search plus file write, patch, managed command, validation, and change transaction tools | admitted task scope | `ChangeSet` |
| `tester-agent` | read/search and `validation_run` | read-only source workspace | `Validation` |
| `reviewer-agent` | read/search plus read-only Git diff/evidence supplied universally | read-only | `Review` |
| `fixer-agent` | Executor mutation tools | unresolved finding path union | `ChangeSet` |

The concrete read set is `repo_inspect`, `file_read`, `artifact_read`, `grep`,
`glob`, `file_search`, and `code_graph`. Executor and Fixer additionally receive
`file_write`, `apply_patch`, `exec_command`, `write_stdin`, `validation_run`,
`change_accept`, and `change_rollback`. Tester receives `validation_run`, but
not general command or file mutation tools.

The loader continues to add registered universal read-only Git and task
bookkeeping tools. Tests inspect the final `AgentProfile.allowed_tools`, not
only Markdown frontmatter. Universal task operations never advance an Agora
stage; only a Host-validated `CognitiveRoleOutput` can do that.

## 6. Bootstrap and capability validation

The native cognitive workflow uses an exact mapping:

```text
Planner  -> planner-agent
Explorer -> explorer-agent
Executor -> executor-agent
Tester   -> tester-agent
Reviewer -> reviewer-agent
Fixer    -> fixer-agent
```

An Executive-local resolver builds the map from fully loaded effective
profiles. It validates:

- all six identities exist and are distinct;
- every requested tool exists after profile loading;
- Planner, Explorer, Tester, and Reviewer have no file/command/change mutation
  tools (Tester may have only the governed validation capability);
- Executor and Fixer contain the required mutation and validation tools;
- the role's output authority remains the canonical
  `CognitiveRoleProfile.required_output_artifacts` contract.

If the native runtime is enabled and any role profile is absent, quarantined,
or violates its contract, daemon bootstrap fails with the role and reason. It
must not report an active role workflow after falling back to `code-agent` or an
alphabetically selected profile.

## 7. Workspace authority

### 7.1 Admission order

AgentControl applies authority in this order:

1. authenticate the parent and attenuate the requested child authority;
2. validate the host-only cognitive binding;
3. normalize the binding scope against the trusted workspace root;
4. narrow the already-attenuated `WorkspacePolicy.writable_roots`;
5. bind the task owner/version in Agora;
6. launch the runtime with that exact narrowed policy.

The binding may only reduce authority. Missing trusted workspace authority,
absolute escape, `..` traversal, symlink escape, an out-of-parent root, or an
empty writer scope fails admission before runtime scheduling. A non-writer must
carry an empty binding scope and receives an empty writable-root set.

### 7.2 Native runtime

For a cognitively bound native Agent, `PrincipalContext.workspace` and
`CapabilityExecutionContext.workspace` use the admitted policy from
`AgentRuntimeInput`. The runtime must not replace it with a policy constructed
from `std::env::current_dir()`.

For a non-cognitive native Agent, an existing trusted policy is likewise
preserved; only a request without any trusted policy may retain the current
working-directory default. This workstream does not broaden either path.

## 8. Finding-bound Fixer scope

`ReviewFinding` gains `affected_paths: Vec<String>`. An unresolved finding is
valid only when every path is normalized, lies in the original task scope, and
the list is non-empty. Historical deserialization may default the new field,
but an empty scope cannot enter a new repair decision.

The workflow computes Fixer authority as follows:

- failed validation: the exact changed paths of the validated `ChangeSet`;
- rejected review: the union of `affected_paths` for the unresolved findings;
- re-review: each preserved finding ID must retain the same affected paths.

The computed paths are written to both `CognitiveTaskNode.workspace_scope` and
`CognitiveTaskRuntimeBinding.workspace_scope`. `AgentTaskPacket.workspace_roots`
continues to describe the overall repository/task roots for read context. The
Fixer gate validates `repair.changed_paths` against the narrower task scope,
not merely the overall workspace roots.

This yields two independent controls:

1. capability execution cannot mutate outside the admitted roots;
2. a fabricated `ChangeSet.changed_paths` outside those roots is rejected by
   the stage gate even if no tool mutation occurred.

## 9. Artifact authority

The existing canonical role contract remains authoritative:

- Planner can return only `Plan`;
- Explorer can return only its investigation/evidence kind;
- Executor and Fixer can return only `ChangeSet`;
- Tester can return only `Validation`;
- Reviewer can return only `Review`.

`CognitiveRoleOutput::validate_for` therefore rejects an Executor-generated
Review before it is committed. Tests retain this behavior explicitly so tool
specialization cannot be mistaken for artifact authority.

## 10. Failure behavior

| Failure | Required result |
|---|---|
| Missing/quarantined/misconfigured role profile | Bootstrap error naming role/profile; no workflow fallback |
| Non-writer requests write or command tool | Profile/tool validation rejects spawn or tool is invisible and refused |
| Cognitive binding exceeds parent workspace | AgentControl returns `Forbidden`; runtime never launches |
| Unresolved finding has no valid affected path | Review/repair decision rejected |
| Fixer requests a path outside finding scope | Admission or capability call rejected; workspace unchanged |
| Executor returns a Review artifact | Host stage gate rejects it; no committed artifact |
| Model prose claims success after a rejected action | No stage advancement; terminal receipt remains failure/blocked |

## 11. Validation strategy

### 11.1 Contract and composition tests

- bundled seeding includes all six Markdown profiles;
- effective profiles have distinct IDs and exact role tool boundaries;
- missing or unsafe role profiles fail the specialized resolver;
- canonical output-kind tests reject cross-role artifact forgery;
- review finding validation rejects empty, escaping, or scope-drifting paths.

### 11.2 Authority tests

- AgentControl narrows Planner/Reviewer bindings to zero writable roots;
- Executor receives only task roots;
- Fixer receives only normalized finding roots;
- native runtime capability context preserves the narrowed policy;
- a write through `file_write`, `apply_patch`, or managed command cannot alter a
  file outside the effective roots.

### 11.3 End-to-end fixture

A deterministic native-role fixture loads the shipped profiles, admits role
packets, invokes actual registered capabilities with a scripted LLM, waits for
the authoritative terminal snapshot, and asserts both receipt and filesystem
state. It covers at minimum:

1. Planner and Reviewer attempt a file write: refused, target unchanged;
2. Executor returns a Review: terminal role gate rejects it;
3. Fixer edits one finding path and one sibling path: finding edit may commit,
   sibling edit is refused and the overall out-of-scope attempt cannot PASS.

Tests use temporary repositories and do not depend on prompt wording, language,
repository name, or a fixed checkout path.

### 11.4 Repository and installed acceptance

Rust tests and formatting run only through `scripts/cargo-agent.sh`, using the
narrowest relevant packages and targets. Because this work changes profiles,
configuration bootstrap, tool authority, and client/daemon behavior, completion
also requires:

```bash
sudo bash scripts/aletheon.sh deploy
```

The installed gate must prove identical release/installed/running executable
digests, stable machine and user daemon restart counters, and a real request
through `/usr/bin/aletheon` and the official user socket.

## 12. Acceptance criteria

This workstream is complete only when:

1. all six specialized effective profiles are distinct in the loaded registry;
2. no native cognitive role silently falls back to `code-agent`;
3. Planner and Reviewer cannot modify the fixture through any exposed tool;
4. Tester can run governed validation but cannot modify production sources;
5. Executor can mutate only the admitted task scope and cannot emit Review;
6. Fixer can mutate only paths bound to every explicit unresolved finding;
7. invalid profile, workspace, finding, artifact, and tool requests fail closed;
8. deterministic end-to-end fixtures verify terminal receipts and final files;
9. focused checks, formatting, and `git diff --check` pass;
10. system-installed deployment acceptance passes.
