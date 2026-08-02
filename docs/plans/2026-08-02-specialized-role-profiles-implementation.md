# Specialized Role Profiles Implementation Plan

> **For agentic workers:** Use `flow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Enforce six distinct native cognitive role profiles so read-only roles cannot mutate the workspace, Executor cannot forge review output, and Fixer can write only paths attached to explicit findings.

**Architecture:** Load six bundled Markdown profiles into the effective profile registry, resolve each cognitive role to exactly one profile, and fail daemon bootstrap when the mapping or final tool set is invalid. Carry the role's typed workspace scope through AgentControl into Native Cognit, attenuate it against the parent's trusted workspace, and bind Fixer authority to typed finding paths. Verify the boundary with deterministic runtime fixtures, terminal receipts, final filesystem state, and the installed system runtime.

**Tech Stack:** Rust, Tokio, Serde, Markdown agent profiles, AgentControl, Native Cognit, scoped filesystem tools, shell-based installed-runtime acceptance.

---

## Requirement anchors

- Specialized role/tool boundaries: `docs/plans/Aletheon_Engineering_Capability_Gap_Audit_2026-08-01.md:416-427`.
- Mandatory negative acceptance cases: `docs/plans/Aletheon_Engineering_Capability_Gap_Audit_2026-08-01.md:470-472`.
- E2E, receipt, observable fallback, and Host acceptance rules: `docs/plans/Aletheon_Engineering_Capability_Gap_Audit_2026-08-01.md:560-569`.
- Approved ten-item acceptance contract: `docs/plans/2026-08-02-specialized-role-profiles-design.md:256-269`.
- Current shared `code-agent` fallback: `crates/executive/src/host/daemon/bootstrap/services.rs:173-202`.
- Current Fixer whole-task scope and root-level validation: `crates/executive/src/application/cognitive_role_workflow.rs:821-900,1124-1152`.
- Current Native Cognit reconstruction of workspace authority: `crates/executive/src/adapters/runtime/native_cognit.rs:249-290,808-827`.

## File map

| Path | Action | Responsibility |
|---|---|---|
| `agents/planner-agent.md` | Create | Read-only planning profile |
| `agents/explorer-agent.md` | Create | Read-only repository investigation profile |
| `agents/executor-agent.md` | Create | Task-scoped mutation and validation profile |
| `agents/tester-agent.md` | Create | Read-only validation profile |
| `agents/reviewer-agent.md` | Create | Read-only diff and evidence profile |
| `agents/fixer-agent.md` | Create | Finding-scoped repair profile |
| `crates/executive/src/host/daemon/bootstrap/role_profiles.rs` | Create | Exact role-to-profile resolver and tool-contract validation |
| `crates/executive/src/host/daemon/bootstrap/mod.rs` | Modify | Register the resolver module |
| `crates/executive/src/host/daemon/bootstrap/services.rs` | Modify | Replace shared fallback with the validated resolver |
| `crates/executive/src/host/daemon/bootstrap/runtime.rs` | Modify | Reuse the universal-tool definition during contract validation |
| `crates/executive/src/host/daemon/bootstrap/bundled_profiles.rs` | Modify | Embed and seed all six role profiles |
| `crates/fabric/src/types/local_authority.rs` | Modify | Safely narrow trusted workspace policy to declared paths |
| `crates/fabric/src/types/cognitive_workflow.rs` | Modify | Add typed paths to review findings |
| `crates/fabric/tests/local_authority_contract.rs` | Modify | Prove traversal and symlink escapes fail closed |
| `crates/executive/src/application/agent_control/mod.rs` | Modify | Apply cognitive workspace attenuation before admission |
| `crates/executive/src/adapters/runtime/native_cognit.rs` | Modify | Preserve the admitted workspace in tool execution context |
| `crates/executive/src/application/cognitive_role_workflow.rs` | Modify | Compute Fixer scopes and enforce artifact/path gates |
| `crates/executive/tests/agent_cognitive_admission.rs` | Modify | Assert admitted bindings become effective authority |
| `crates/executive/tests/native_cognit_runtime.rs` | Modify | Assert Native Cognit uses admitted workspace policy |
| `crates/executive/tests/specialized_role_capabilities.rs` | Create | Deterministic boundary E2E and terminal receipt fixture |

## Tool contracts

The resolver validates final effective tools after universal tools are merged. Each set below is exact: any missing or extra non-universal tool is a bootstrap error.

| Role | Profile ID | Explicit tools |
|---|---|---|
| Planner | `planner-agent` | `repo_inspect`, `file_read`, `artifact_read`, `grep`, `glob`, `file_search`, `code_graph` |
| Explorer | `explorer-agent` | `repo_inspect`, `file_read`, `artifact_read`, `grep`, `glob`, `file_search`, `code_graph` |
| Executor | `executor-agent` | Read set plus `file_write`, `apply_patch`, `exec_command`, `write_stdin`, `validation_run`, `change_accept`, `change_rollback` |
| Tester | `tester-agent` | Read set plus `validation_run` |
| Reviewer | `reviewer-agent` | Read set |
| Fixer | `fixer-agent` | Executor set |

Although Planner, Explorer, and Reviewer share an explicit tool set, their profile IDs, prompts, responsibilities, required artifact types, and effective registry entries remain distinct.

### Task 1: Bundle six role-specific Markdown profiles

**Files:**
- Create: `agents/planner-agent.md`
- Create: `agents/explorer-agent.md`
- Create: `agents/executor-agent.md`
- Create: `agents/tester-agent.md`
- Create: `agents/reviewer-agent.md`
- Create: `agents/fixer-agent.md`
- Modify: `crates/executive/src/host/daemon/bootstrap/bundled_profiles.rs`
- Test: `crates/executive/src/host/daemon/bootstrap/bundled_profiles.rs`

- [x] **Step 1: Add a failing bundled-profile inventory test**

Add a unit test beside the existing bundled asset tests:

```rust
#[test]
fn specialized_role_profiles_are_bundled() {
    let expected = [
        "planner-agent",
        "explorer-agent",
        "executor-agent",
        "tester-agent",
        "reviewer-agent",
        "fixer-agent",
    ];

    for id in expected {
        let filename = format!("{id}.md");
        let (_, markdown) = PROFILES
            .iter()
            .find(|(name, _)| *name == filename)
            .unwrap_or_else(|| panic!("missing bundled profile {id}"));
        assert!(markdown.contains(&format!("name: {id}")));
    }
}
```

- [x] **Step 2: Run the narrow test and observe the missing assets**

Run:

```bash
bash scripts/cargo-agent.sh test -p executive host::daemon::bootstrap::bundled_profiles::tests::specialized_role_profiles_are_bundled -- --exact
```

Expected: FAIL because the `PROFILES` table has no `planner-agent.md` entry.

- [x] **Step 3: Add the six complete profile documents**

Use the existing agent-profile front matter schema. The Planner document is:

```markdown
---
name: planner-agent
description: Produces a typed task graph without modifying the workspace.
tools: [repo_inspect, file_read, artifact_read, grep, glob, file_search, code_graph]
max_iterations: 20
role: Leaf
---

You are the Planner cognitive role. Read the admitted evidence and return only
the required typed Plan artifact. Do not request or simulate mutation. Every
task edge and acceptance statement must cite projected evidence.
```

Create `explorer-agent.md`, `tester-agent.md`, and `reviewer-agent.md` with `max_iterations: 20`, `role: Leaf`, and the same inline seven-read-tool list shown for Planner. Tester uses the inline list `[repo_inspect, file_read, artifact_read, grep, glob, file_search, code_graph, validation_run]`. Use these exact identity fields and bodies:

```markdown
name: explorer-agent
description: Verifies repository paths and symbols without modifying the workspace.
body: Return only typed Investigation evidence. Read repository content, verify every claimed path or symbol, and never mutate files.

name: tester-agent
description: Runs governed validation without modifying production sources.
body: Return only typed Validation evidence. Run only admitted governed validation and never modify production sources.

name: reviewer-agent
description: Independently reviews diffs and evidence without mutation authority.
body: Return only a typed Review. Inspect diffs and evidence independently, attach affected paths to each finding, and never invoke mutation tools.
```

Create `executor-agent.md` and `fixer-agent.md` with `max_iterations: 20`, `role: Leaf`, and this exact inline tool value:

```yaml
tools: [repo_inspect, file_read, artifact_read, grep, glob, file_search, code_graph, file_write, apply_patch, exec_command, write_stdin, validation_run, change_accept, change_rollback]
```

Use these exact identity fields and bodies:

```markdown
name: executor-agent
description: Produces task-scoped changes and governed validation evidence.
body: Return only a typed ChangeSet. Modify only the admitted task workspace scope, record every changed path, and never emit Review or Validation authority.

name: fixer-agent
description: Repairs only paths bound to explicit unresolved findings.
body: Return only a typed ChangeSet for every supplied unresolved finding. Modify only the bound finding paths and cite each finding ID.
```

- [x] **Step 4: Add every profile to the existing bundled asset table**

Follow the existing `(filename, include_str!)` pattern in `crates/executive/src/host/daemon/bootstrap/bundled_profiles.rs`. Add these exact entries to `PROFILES`:

```rust
("planner-agent.md", include_str!("../../../../../../agents/planner-agent.md")),
("explorer-agent.md", include_str!("../../../../../../agents/explorer-agent.md")),
("executor-agent.md", include_str!("../../../../../../agents/executor-agent.md")),
("tester-agent.md", include_str!("../../../../../../agents/tester-agent.md")),
("reviewer-agent.md", include_str!("../../../../../../agents/reviewer-agent.md")),
("fixer-agent.md", include_str!("../../../../../../agents/fixer-agent.md")),
```

- [x] **Step 5: Re-run the inventory test**

```bash
bash scripts/cargo-agent.sh test -p executive host::daemon::bootstrap::bundled_profiles::tests::specialized_role_profiles_are_bundled -- --exact
```

Expected: PASS; all six bundled Markdown profiles are readable by exact ID.

- [x] **Step 6: Commit the bundled profile stage**

```bash
git add agents/planner-agent.md agents/explorer-agent.md agents/executor-agent.md agents/tester-agent.md agents/reviewer-agent.md agents/fixer-agent.md crates/executive/src/host/daemon/bootstrap/bundled_profiles.rs
git diff --cached --check
git commit -F - <<'MSG'
feat(agents): bundle specialized role profiles

Native cognitive roles need separate identities and explicit tool grants rather
than one shared code-agent profile.

- add six role-specific Markdown profiles
- bundle each profile into release assets
- verify exact profile IDs through an inventory test
MSG
```

### Task 2: Resolve role profiles exactly and fail bootstrap closed

**Files:**
- Create: `crates/executive/src/host/daemon/bootstrap/role_profiles.rs`
- Modify: `crates/executive/src/host/daemon/bootstrap/mod.rs`
- Modify: `crates/executive/src/host/daemon/bootstrap/services.rs:173-202`
- Modify: `crates/executive/src/host/daemon/bootstrap/runtime.rs:47-59`
- Test: `crates/executive/src/host/daemon/bootstrap/role_profiles.rs`

- [x] **Step 1: Write resolver tests before implementation**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_cognitive_role_has_a_distinct_profile_id() {
        let ids = ROLE_PROFILE_IDS.map(|(_, id)| id);
        let unique = ids.into_iter().collect::<std::collections::HashSet<_>>();
        assert_eq!(unique.len(), 6);
    }

    #[test]
    fn read_only_roles_reject_mutation_tools() {
        for role in [CognitiveRole::Planner, CognitiveRole::Explorer, CognitiveRole::Tester, CognitiveRole::Reviewer] {
            let permitted = permitted_tools(role);
            for forbidden in ["file_write", "apply_patch", "exec_command", "write_stdin", "change_accept", "change_rollback"] {
                assert!(!permitted.contains(forbidden), "{role:?} admitted {forbidden}");
            }
        }
        assert!(permitted_tools(CognitiveRole::Tester).contains("validation_run"));
    }
}
```

- [x] **Step 2: Run the test and observe unresolved resolver symbols**

Run:

```bash
bash scripts/cargo-agent.sh test -p executive host::daemon::bootstrap::role_profiles::tests --lib
```

Expected: FAIL to compile because `ROLE_PROFILE_IDS` and `permitted_tools` do not exist.

- [x] **Step 3: Implement the exact mapping and permitted sets**

Create `role_profiles.rs` with constants for the read/write sets and this mapping:

```rust
use std::collections::{HashMap, HashSet};

use fabric::cognitive_workflow::CognitiveRole;

use crate::application::cognitive_role_workflow::RoleLaunchProfile;
use fabric::AgentProfile;

pub(super) const ROLE_PROFILE_IDS: [(CognitiveRole, &str); 6] = [
    (CognitiveRole::Planner, "planner-agent"),
    (CognitiveRole::Explorer, "explorer-agent"),
    (CognitiveRole::Executor, "executor-agent"),
    (CognitiveRole::Tester, "tester-agent"),
    (CognitiveRole::Reviewer, "reviewer-agent"),
    (CognitiveRole::Fixer, "fixer-agent"),
];

fn permitted_tools(role: CognitiveRole) -> HashSet<&'static str> {
    let mut permitted = explicit_tools(role).into_iter().collect::<HashSet<_>>();
    permitted.extend(super::runtime::UNIVERSAL_TOOLS.iter().copied());
    permitted
}
```

Define `READ_TOOLS` and `WRITE_TOOLS` from the Tool contracts table. `explicit_tools` returns `READ_TOOLS` for Planner, Explorer, and Reviewer; `READ_TOOLS + validation_run` for Tester; and `READ_TOOLS + WRITE_TOOLS` for Executor and Fixer. Root is rejected because it is not a launchable native worker role.

- [x] **Step 4: Implement final effective-profile validation**

```rust
pub(super) fn resolve_role_launch_profiles(
    profiles: &HashMap<String, AgentProfile>,
) -> anyhow::Result<HashMap<CognitiveRole, RoleLaunchProfile>> {
    let mut launches = HashMap::new();
    for (role, expected_id) in ROLE_PROFILE_IDS {
        let profile = profiles
            .get(expected_id)
            .ok_or_else(|| anyhow::anyhow!("missing required cognitive profile '{expected_id}'"))?;
        anyhow::ensure!(profile.id.0 == expected_id, "cognitive profile identity mismatch for {role:?}");

        let actual = profile.allowed_tools.iter().map(String::as_str).collect::<HashSet<_>>();
        let expected = permitted_tools(role);
        anyhow::ensure!(
            actual == expected,
            "cognitive profile '{expected_id}' effective tool contract mismatch"
        );
        launches.insert(role, RoleLaunchProfile {
            profile_id: profile.id.clone(),
            allowed_tools: profile.allowed_tools.clone(),
        });
    }
    Ok(launches)
}
```

Use the existing `fabric::AgentProfile` and `RoleLaunchProfile` types; do not add a second profile or factory type.

- [x] **Step 5: Replace the shared fallback at bootstrap**

Register `mod role_profiles;` in `bootstrap/mod.rs`. In `services.rs`, when Native Cognit is installed, call `resolve_role_launch_profiles(&agent_profiles_for_tools)?` and pass the returned map to the existing five-argument `RoleWorkflowFactory::new`; otherwise retain the current absence behavior. Delete the `code-agent` lookup, alphabetical fallback, profile cloning, and `Option::map` path entirely.

Add tests using six in-memory final `AgentProfile` values:

```rust
#[test]
fn resolver_rejects_missing_or_overpowered_profile() {
    let mut profiles = exact_test_profiles();
    profiles.remove("reviewer-agent");
    assert!(resolve_role_launch_profiles(&profiles).unwrap_err().to_string().contains("reviewer-agent"));

    let mut profiles = exact_test_profiles();
    profiles.get_mut("planner-agent").unwrap().allowed_tools.push("file_write".into());
    assert!(resolve_role_launch_profiles(&profiles).unwrap_err().to_string().contains("tool contract mismatch"));
}
```

- [x] **Step 6: Run resolver and bootstrap tests**

Run:

```bash
bash scripts/cargo-agent.sh test -p executive host::daemon::bootstrap::role_profiles::tests --lib
bash scripts/cargo-agent.sh test -p executive host::daemon::bootstrap --lib
```

Expected: PASS; missing, renamed, underpowered, or overpowered role profiles fail closed and no code-agent fallback remains.

- [x] **Step 7: Commit the resolver stage**

```bash
git add crates/executive/src/host/daemon/bootstrap/role_profiles.rs crates/executive/src/host/daemon/bootstrap/mod.rs crates/executive/src/host/daemon/bootstrap/services.rs crates/executive/src/host/daemon/bootstrap/runtime.rs
git diff --cached --check
git commit -F - <<'MSG'
feat(agents): enforce exact cognitive profile contracts

Shared profile fallback makes role labels cosmetic and may silently broaden
worker authority.

- map every native cognitive role to one required profile ID
- validate the final effective tool set after universal grants
- fail daemon bootstrap instead of falling back to code-agent
MSG
```

### Task 3: Add symlink-safe workspace path attenuation

**Files:**
- Modify: `crates/fabric/src/types/local_authority.rs:111-140`
- Test: `crates/fabric/tests/local_authority_contract.rs`

- [x] **Step 1: Add failing contract tests**

```rust
#[test]
fn declared_paths_are_resolved_inside_existing_authority() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("src")).unwrap();
    let policy = WorkspacePolicy::from_resolved_roots(
        temp.path().to_path_buf(),
        vec![temp.path().to_path_buf()],
    ).unwrap();

    let narrowed = policy.narrow_to_declared_paths(&["src/lib.rs".into()]).unwrap();
    assert_eq!(narrowed.writable_roots(), &[temp.path().join("src/lib.rs")]);
}

#[test]
fn declared_paths_reject_parent_traversal() {
    let temp = tempfile::tempdir().unwrap();
    let policy = WorkspacePolicy::from_resolved_roots(
        temp.path().to_path_buf(),
        vec![temp.path().to_path_buf()],
    ).unwrap();
    assert!(policy.narrow_to_declared_paths(&["../escape".into()]).is_err());
}

#[cfg(unix)]
#[test]
fn declared_paths_reject_symlink_escape() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), root.path().join("link")).unwrap();
    let policy = WorkspacePolicy::from_resolved_roots(
        root.path().to_path_buf(),
        vec![root.path().to_path_buf()],
    ).unwrap();
    assert!(policy.narrow_to_declared_paths(&["link/pwned.rs".into()]).is_err());
}
```

- [x] **Step 2: Run the Fabric contract target**

Run:

```bash
bash scripts/cargo-agent.sh test -p fabric --test local_authority_contract
```

Expected: FAIL to compile because `narrow_to_declared_paths` does not exist.

- [x] **Step 3: Implement canonical containment without creating files**

Add:

```rust
pub fn narrow_to_declared_paths(mut self, declared: &[String]) -> Result<Self, String> {
    let mut resolved = Vec::with_capacity(declared.len());
    for raw in declared {
        let relative = std::path::Path::new(raw);
        if raw.trim().is_empty() || relative.components().any(|part| matches!(part, std::path::Component::ParentDir)) {
            return Err(format!("invalid declared workspace path: {raw}"));
        }
        let candidate = if relative.is_absolute() {
            relative.to_path_buf()
        } else {
            self.cwd.join(relative)
        };
        let canonical = resolve_existing_ancestor(&candidate)?;
        if !self.writable_roots.iter().any(|root| canonical.starts_with(root)) {
            return Err(format!("declared workspace path exceeds existing authority: {raw}"));
        }
        resolved.push(canonical);
    }
    self.narrow_writable_roots(resolved)
}

fn resolve_existing_ancestor(candidate: &std::path::Path) -> Result<PathBuf, String> {
    let mut cursor = candidate;
    let mut missing = Vec::new();
    while !cursor.exists() {
        let name = cursor.file_name().ok_or_else(|| {
            format!("declared workspace path has no existing ancestor: {}", candidate.display())
        })?;
        missing.push(name.to_os_string());
        cursor = cursor.parent().ok_or_else(|| {
            format!("declared workspace path has no parent: {}", candidate.display())
        })?;
    }
    let mut resolved = cursor.canonicalize().map_err(|error| {
        format!("failed to resolve workspace path {}: {error}", cursor.display())
    })?;
    for component in missing.into_iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
}
```

Keep both methods in the existing `WorkspacePolicy` module. The public method rejects `..` before the helper runs; the helper preserves a non-existent target path while resolving every existing symlink before the containment check.

- [x] **Step 4: Add duplicate, absolute, and empty-scope assertions**

Extend the same tests to prove duplicate paths collapse, an admitted absolute path succeeds, an absolute outside path fails, and `narrow_to_declared_paths(&[])` produces a read-only policy with no writable roots.

- [x] **Step 5: Run Fabric tests**

Run:

```bash
bash scripts/cargo-agent.sh test -p fabric --test local_authority_contract
bash scripts/cargo-agent.sh test -p fabric local_authority --lib
```

Expected: PASS; relative and missing paths resolve safely while traversal and symlink escapes are rejected.

- [x] **Step 6: Commit workspace attenuation**

```bash
git add crates/fabric/src/types/local_authority.rs crates/fabric/tests/local_authority_contract.rs
git diff --cached --check
git commit -F - <<'MSG'
feat(authority): narrow workspace to declared paths

Cognitive role scopes need a reusable host-authority primitive that cannot be
bypassed with relative traversal or existing symlinks.

- resolve declared paths against the trusted workspace
- preserve missing targets while canonicalizing existing ancestors
- reject paths outside parent write authority
MSG
```

### Task 4: Apply cognitive workspace authority before runtime launch

**Files:**
- Modify: `crates/executive/src/application/agent_control/mod.rs:882-940`
- Modify: `crates/executive/src/adapters/runtime/native_cognit.rs:249-290,808-827`
- Test: `crates/executive/tests/agent_cognitive_admission.rs`
- Test: `crates/executive/tests/native_cognit_runtime.rs`

- [x] **Step 1: Add an AgentControl admission test that records runtime authority**

Extend the existing recording launcher fixture so the launched `AgentRuntimeInput.workspace` can be inspected, then add:

```rust
#[tokio::test]
async fn cognitive_binding_attenuates_effective_runtime_workspace() {
    let fixture = AdmissionFixture::new().await;
    let allowed = fixture.root.path().join("src/allowed.rs");
    let sibling = fixture.root.path().join("src/sibling.rs");
    std::fs::create_dir_all(allowed.parent().unwrap()).unwrap();
    std::fs::write(&allowed, "allowed").unwrap();
    std::fs::write(&sibling, "sibling").unwrap();

    let binding = fixture.binding(CognitiveRole::Fixer, vec!["src/allowed.rs".into()]);
    fixture.spawn_with_binding(binding).await.unwrap();
    let workspace = fixture.recorded_workspace().await.unwrap();
    assert_eq!(workspace.writable_roots(), &[allowed]);
    assert!(!workspace.writable_roots().iter().any(|path| path == &sibling));
}
```

Add a second test with Planner and an empty `workspace_scope`; assert the launched workspace exists but has an empty `writable_roots()` list. Add negative tests for a writer with an empty scope, a scope outside the trusted workspace, and a cognitive request with no trusted workspace.

- [x] **Step 2: Run the admission test and observe whole-workspace authority**

Run:

```bash
bash scripts/cargo-agent.sh test -p executive --test agent_cognitive_admission
```

Expected: FAIL because `spawn` forwards the trusted whole workspace without narrowing it to the cognitive binding.

- [x] **Step 3: Add a host-only cognitive attenuation helper**

In `application/agent_control/mod.rs`, add:

```rust
fn constrain_cognitive_workspace(request: &mut AgentSpawnRequest) -> Result<(), AgentControlError> {
    let Some(binding) = request.cognitive_binding.as_ref() else {
        return Ok(());
    };
    let workspace = request.trusted_workspace.take().ok_or_else(|| {
        control_error(
            AgentControlErrorKind::Forbidden,
            "cognitive Agent spawn has no trusted workspace authority",
        )
    })?;
    if binding.role.can_write_workspace() && binding.workspace_scope.is_empty() {
        return Err(control_error(
            AgentControlErrorKind::Forbidden,
            "writable cognitive role has an empty workspace scope",
        ));
    }
    let scope = if binding.role.can_write_workspace() {
        binding.workspace_scope.as_slice()
    } else {
        &[]
    };
    request.trusted_workspace = Some(
        workspace
            .narrow_to_declared_paths(scope)
            .map_err(AgentControlError::invalid)?,
    );
    Ok(())
}
```

Call it after parent authority attenuation and its second `request.validate()`, but before `agent_spawn_request_hash`, admission reservation, persistence, or runtime launch. Run `request.validate()?` again after narrowing so the admitted request and its hash represent the final authority.

- [x] **Step 4: Add a failing Native Cognit workspace preservation test**

In `native_cognit_runtime.rs`, launch a cognitive runtime input whose `workspace` has only `src/allowed.rs` writable. Register the real scoped `file_write` tool and assert its `CapabilityExecutionContext.workspace.writable_roots()` is exactly the admitted file, not the process current directory. Also pass `workspace: None` with a cognitive binding and assert `AgentControlErrorKind::Forbidden`.

Run:

```bash
bash scripts/cargo-agent.sh test -p executive --test native_cognit_runtime
```

Expected: FAIL because `agent_principal_context` reconstructs authority from `std::env::current_dir()`.

- [x] **Step 5: Preserve admitted workspace in Native Cognit**

Change the helper to:

```rust
fn agent_principal_context(
    agent_id: String,
    admitted_workspace: Option<WorkspacePolicy>,
    cognitive_binding: Option<&CognitiveTaskRuntimeBinding>,
) -> Result<PrincipalContext, AgentControlError> {
    let workspace = match (admitted_workspace, cognitive_binding) {
        (Some(workspace), _) => workspace,
        (None, Some(_)) => {
            return Err(control_error(
                AgentControlErrorKind::Forbidden,
                "cognitive runtime has no admitted workspace authority",
            ));
        }
        (None, None) => WorkspacePolicy::from_resolved_roots(
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/tmp")),
            Vec::new(),
        ).map_err(runtime_failure)?,
    };
    let uid = nix::unistd::Uid::effective().as_raw();
    Ok(PrincipalContext::new(
        PrincipalId::local_uid(uid),
        LocalOsPrincipal { uid, gid: nix::unistd::Gid::effective().as_raw() },
        ConnectionId::new(),
        ThreadId(agent_id),
        workspace,
        PermissionProfileId::workspace_write(),
        ApprovalPolicy::OnRequest,
    ))
}
```

Call it with `input.workspace.clone()` and `input.request.cognitive_binding.as_ref()`. Continue copying `principal_context.workspace` into `CapabilityExecutionContext`; do not construct another policy later in the turn.

- [x] **Step 6: Run both runtime authority targets**

```bash
bash scripts/cargo-agent.sh test -p executive --test agent_cognitive_admission
bash scripts/cargo-agent.sh test -p executive --test native_cognit_runtime
```

Expected: PASS; AgentControl admission and the actual capability context expose the same narrowed workspace.

- [x] **Step 7: Commit runtime authority preservation**

```bash
git add crates/executive/src/application/agent_control/mod.rs crates/executive/src/adapters/runtime/native_cognit.rs crates/executive/tests/agent_cognitive_admission.rs crates/executive/tests/native_cognit_runtime.rs
git diff --cached --check
git commit -F - <<'MSG'
fix(agents): preserve cognitive workspace authority

Role bindings were recorded but Native Cognit rebuilt write authority from the
daemon working directory before invoking tools.

- attenuate cognitive scope before admission and request hashing
- reject writable roles without trusted task scope
- carry the admitted policy into capability execution
MSG
```

### Task 5: Bind Fixer authority to typed finding paths

**Files:**
- Modify: `crates/fabric/src/types/cognitive_workflow.rs:500-514`
- Modify: `crates/executive/src/application/cognitive_role_workflow.rs:540-900,1124-1152`
- Test: `crates/executive/src/application/cognitive_role_workflow.rs:1171-1300`

- [x] **Step 1: Add affected paths to the typed finding schema**

Change the type to:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewFinding {
    pub id: String,
    pub severity: String,
    pub summary: String,
    pub evidence_refs: Vec<String>,
    #[serde(default)]
    pub affected_paths: Vec<String>,
    pub resolved: bool,
}
```

Update the two existing `ReviewFinding` constructors in the workflow tests with explicit `affected_paths`. Use `vec!["src/allowed.rs".into()]` for unresolved findings and an empty vector only for already-resolved legacy fixtures.

- [x] **Step 2: Add failing finding-scope and artifact tests**

Add workflow tests that assert:

```rust
#[tokio::test]
async fn unresolved_review_finding_requires_admitted_paths() {
    let mut fixture = AcceptanceFixture::review_failure();
    fixture.reviewer_output.findings[0].affected_paths.clear();
    let error = fixture.run().await.unwrap_err();
    assert!(error.to_string().contains("finding has no affected paths"));
}

#[tokio::test]
async fn fixer_change_set_must_stay_inside_finding_scope() {
    let mut fixture = AcceptanceFixture::review_failure();
    fixture.reviewer_output.findings[0].affected_paths = vec!["src/allowed.rs".into()];
    fixture.fixer_output.changed_paths = vec!["src/sibling.rs".into()];
    let error = fixture.run().await.unwrap_err();
    assert!(error.to_string().contains("outside the finding scope"));
}
```

Add a launch-recording assertion that the Fixer `AgentTaskPacket.task.workspace_scope`, `packet.workspace_roots`, and `CognitiveTaskRuntimeBinding.workspace_scope` are all exactly `src/allowed.rs`. Add an Executor test returning `CognitiveArtifact::Review` and assert `CognitiveRoleOutput::validate_for` rejects it with `role output kind is not authorized`.

- [x] **Step 3: Run the workflow tests and observe whole-task scope**

Run:

```bash
bash scripts/cargo-agent.sh test -p executive application::cognitive_role_workflow::tests --lib
```

Expected: FAIL because findings have no paths and `invoke_acceptance_role` assigns the request's whole task scope to every writer.

- [x] **Step 4: Validate and normalize unresolved finding paths**

Add a helper in `cognitive_role_workflow.rs`:

```rust
fn unresolved_finding_scope(
    findings: &[ReviewFinding],
    task_roots: &[String],
) -> anyhow::Result<Vec<String>> {
    let mut scope = Vec::new();
    for finding in findings.iter().filter(|finding| !finding.resolved) {
        anyhow::ensure!(
            !finding.affected_paths.is_empty(),
            "unresolved finding has no affected paths: {}",
            finding.id
        );
        for path in &finding.affected_paths {
            anyhow::ensure!(
                path_is_within_roots(path, task_roots),
                "finding path is outside the owned task scope: {path}"
            );
            if !scope.contains(path) {
                scope.push(path.clone());
            }
        }
    }
    scope.sort();
    Ok(scope)
}
```

When re-reviewing, require matching finding IDs to preserve their `affected_paths`; a role cannot silently widen an existing finding. Reject duplicate finding IDs and empty IDs before computing scope.

- [x] **Step 5: Pass explicit write scope into acceptance role launches**

Change the signature to:

```rust
async fn invoke_acceptance_role(
    &self,
    request: &AcceptanceWorkflowRequest,
    version: u64,
    owner: ProcessId,
    role: CognitiveRole,
    write_scope: Option<Vec<String>>,
) -> anyhow::Result<PreparedRole>
```

Compute the effective role scope once:

```rust
let role_scope = match (role.can_write_workspace(), write_scope) {
    (false, None) => Vec::new(),
    (false, Some(_)) => anyhow::bail!("read-only role received a write scope"),
    (true, Some(scope)) if !scope.is_empty() => scope,
    (true, _) => anyhow::bail!("writable role received no bounded write scope"),
};
```

Use `role_scope.clone()` for `task.workspace_scope`, `packet.workspace_roots`, and `CognitiveTaskRuntimeBinding.workspace_scope`. Call sites use:

- Tester and Reviewer: `None`.
- Validation-failure Fixer: `Some(latest_change_set.changed_paths.clone())`.
- Review-failure Fixer: `Some(unresolved_finding_scope(&unresolved, &request.workspace_scope)?)`.

- [x] **Step 6: Enforce Fixer output against the packet's exact scope**

Replace the current `packet.workspace_roots` comparison in `validate_fixer_artifact` with:

```rust
anyhow::ensure!(
    repair.changed_paths.iter().all(|path| {
        packet.task.workspace_scope.iter().any(|allowed| path == allowed)
    }),
    "fixer changed a path outside the finding scope"
);
```

Keep the existing transaction, workspace version, diff reference, and `finding:{id}` evidence checks. Require every unresolved finding ID to be cited; a narrower output path list is valid, but no output path may be outside the exact binding.

- [x] **Step 7: Run Fabric and workflow targets**

```bash
bash scripts/cargo-agent.sh test -p fabric cognitive_workflow --lib
bash scripts/cargo-agent.sh test -p executive application::cognitive_role_workflow::tests --lib
```

Expected: PASS; invalid finding paths and forged artifact kinds fail before candidate commit, and Fixer receives the exact typed path union.

- [x] **Step 8: Commit typed finding scope enforcement**

```bash
git add crates/fabric/src/types/cognitive_workflow.rs crates/executive/src/application/cognitive_role_workflow.rs
git diff --cached --check
git commit -F - <<'MSG'
feat(agents): bind repairs to review finding paths

Finding IDs alone prove why a repair ran but do not bound which files the
Fixer may change.

- attach admitted paths to typed review findings
- propagate exact path unions into Fixer runtime bindings
- reject changes and rereviews that exceed the original finding scope
MSG
```

### Task 6: Add deterministic native-role boundary acceptance

**Files:**
- Create: `crates/executive/tests/specialized_role_capabilities.rs`

- [x] **Step 1: Build a deterministic fixture over the real boundaries**

Create an integration fixture that uses:

- the existing `ScriptedLlm` pattern from `crates/executive/tests/native_cognit_runtime.rs:43-102`;
- the real `NativeCognitRuntime`, `AgentControlPort`, profile registry, and scoped filesystem capability path;
- a temporary repository containing `src/allowed.rs` and `src/sibling.rs`;
- `AgentWaitRequest` after every spawn, so only an authoritative terminal snapshot is accepted;
- a captured `CognitiveRoleOutput` receipt plus final bytes read directly from disk.

The fixture's public test API is:

```rust
struct RoleBoundaryFixture {
    root: tempfile::TempDir,
    control: Arc<dyn AgentControlPort>,
    root_agent_id: AgentId,
}

struct BoundaryResult {
    terminal: AgentSnapshot,
    output: Option<CognitiveRoleOutput>,
    validation_error: Option<String>,
    tool_denials: Vec<String>,
    allowed_bytes: Vec<u8>,
    sibling_bytes: Vec<u8>,
}

impl RoleBoundaryFixture {
    async fn new(role: CognitiveRole, scope: Vec<String>, script: Vec<LlmResponse>) -> Self;
    async fn run(self) -> BoundaryResult;
}
```

`run` must spawn through AgentControl, then call:

```rust
let terminal = self.control.wait(AgentWaitRequest {
    caller_root_agent_id: self.root_agent_id,
    agent_id: handle.agent_id,
    timeout_ms: 5_000,
}).await.expect("terminal cognitive snapshot");
assert!(terminal.status.is_terminal());
```

Deserialize the durable result receipt into `CognitiveRoleOutput`, validate it against the original packet, and only then read both fixture files. Do not treat an asynchronous tool return or child prose as terminal success.

- [x] **Step 2: Add read-only role mutation cases**

```rust
#[tokio::test]
async fn planner_reviewer_and_tester_cannot_write() {
    for role in [CognitiveRole::Planner, CognitiveRole::Reviewer, CognitiveRole::Tester] {
        let result = RoleBoundaryFixture::new(
            role,
            Vec::new(),
            script_calling_file_write("src/allowed.rs", "mutated\n"),
        ).await.run().await;
        assert!(result.terminal.status.is_terminal());
        assert_eq!(result.allowed_bytes, b"original\n", "{role:?} modified the file");
        assert!(result.tool_denials.iter().any(|message| message.contains("file_write")));
    }
}
```

Implement `script_calling_file_write` as two deterministic `LlmResponse` values: the first contains one `ToolUse` block named `file_write` with JSON `{ "path": "src/allowed.rs", "content": "mutated\n" }`; the second contains the role's typed completion output. The test passes only when the tool is unavailable or authority rejects it and the file bytes remain unchanged. For Tester, add a separate successful `validation_run` script to prove governed validation remains exposed.

- [x] **Step 3: Add Executor artifact-forgery and task-scope cases**

```rust
#[tokio::test]
async fn executor_cannot_commit_review_or_escape_task_scope() {
    let forged = RoleBoundaryFixture::new(
        CognitiveRole::Executor,
        vec!["src/allowed.rs".into()],
        script_returning_review_artifact(),
    ).await.run().await;
    assert_eq!(
        forged.validation_error.as_deref(),
        Some("role output kind is not authorized")
    );

    let escaped = RoleBoundaryFixture::new(
        CognitiveRole::Executor,
        vec!["src/allowed.rs".into()],
        script_calling_file_write("src/sibling.rs", "mutated\n"),
    ).await.run().await;
    assert!(escaped.tool_denials.iter().any(|message| message.contains("src/sibling.rs")));
    assert_eq!(escaped.sibling_bytes, b"sibling\n");
}
```

The forged receipt must reach `CognitiveRoleOutput::validate_for`; assert the recorded terminal error contains `role output kind is not authorized`. The out-of-scope tool attempt must be present in runtime events and denied by the capability boundary, not removed from the scripted evidence.

- [x] **Step 4: Add exact Fixer finding-scope cases**

```rust
#[tokio::test]
async fn fixer_can_change_only_the_explicit_finding_path() {
    let accepted = RoleBoundaryFixture::new(
        CognitiveRole::Fixer,
        vec!["src/allowed.rs".into()],
        script_calling_file_write_then_change_set(
            "src/allowed.rs",
            "fixed\n",
            vec!["src/allowed.rs".into()],
        ),
    ).await.run().await;
    assert_eq!(accepted.allowed_bytes, b"fixed\n");
    assert_eq!(accepted.sibling_bytes, b"sibling\n");
    assert!(accepted.terminal.status.is_terminal());
    assert!(accepted.validation_error.is_none());

    let rejected = RoleBoundaryFixture::new(
        CognitiveRole::Fixer,
        vec!["src/allowed.rs".into()],
        script_calling_file_write_then_change_set(
            "src/sibling.rs",
            "escaped\n",
            vec!["src/sibling.rs".into()],
        ),
    ).await.run().await;
    assert!(rejected.tool_denials.iter().any(|message| message.contains("src/sibling.rs")));
    assert_eq!(rejected.sibling_bytes, b"sibling\n");
}
```

The successful receipt cites `finding:F-1`, contains a new transaction ID, a nonempty diff artifact reference, and `changed_paths == ["src/allowed.rs"]`. The rejected run cannot produce a PASS terminal status even if its scripted second response claims success.

- [x] **Step 5: Run the boundary test three consecutive times**

```bash
for run in 1 2 3; do
  echo "specialized role boundary run ${run}"
  bash scripts/cargo-agent.sh test -p executive --test specialized_role_capabilities -- --test-threads=1
done
```

Expected: all three runs PASS with identical role verdicts; terminal receipts are observed, Planner/Reviewer/Tester files remain unchanged, Executor Review is rejected, and only the Fixer finding path changes.

- [x] **Step 6: Commit the boundary fixture**

```bash
git add crates/executive/tests/specialized_role_capabilities.rs
git diff --cached --check
git commit -F - <<'MSG'
test(agents): cover specialized role boundaries

Unit-level tool lists do not prove that Native Cognit preserves authority through
spawn, capability execution, typed output validation, and terminal settlement.

- exercise read-only, task-scoped, and finding-scoped roles end to end
- validate authoritative terminal receipts before accepting results
- assert final fixture bytes after both allowed and rejected attempts
MSG
```

### Task 7: Run focused checks and installed-runtime acceptance

**Files:**
- No repository file changes; deployment receipts remain in the existing external acceptance artifact locations.

- [x] **Step 1: Run the complete focused validation set**

Run sequentially; do not run Cargo commands concurrently:

```bash
bash scripts/cargo-agent.sh test -p fabric --test local_authority_contract
bash scripts/cargo-agent.sh test -p fabric cognitive_workflow --lib
bash scripts/cargo-agent.sh test -p executive host::daemon::bootstrap::bundled_profiles::tests::specialized_role_profiles_are_bundled -- --exact
bash scripts/cargo-agent.sh test -p executive host::daemon::bootstrap::role_profiles::tests --lib
bash scripts/cargo-agent.sh test -p executive application::cognitive_role_workflow::tests --lib
bash scripts/cargo-agent.sh test -p executive --test agent_cognitive_admission
bash scripts/cargo-agent.sh test -p executive --test native_cognit_runtime
bash scripts/cargo-agent.sh test -p executive --test specialized_role_capabilities -- --test-threads=1
```

Expected: every command exits 0. A provider error, rejected request, non-terminal child, receipt mismatch, changed forbidden file, or rendered/runtime disagreement is a failure.

- [x] **Step 2: Run formatting and repository diff checks**

```bash
bash scripts/cargo-agent.sh fmt --all -- --check
git diff --check
git status --short
```

Expected: formatting and diff checks exit 0; status contains only intended implementation files plus the two pre-existing untracked audit documents. Never stage or delete either audit document.

- [x] **Step 3: Perform mandatory system-installed deployment acceptance**

```bash
sudo bash scripts/aletheon.sh deploy
```

Expected: PASS. This is the authoritative acceptance and must prove the candidate is installed through the production path, machine and user daemons are active, their restart counters stay stable, and a real governed LLM request completes through `/usr/bin/aletheon` and the official user socket. A development binary, isolated daemon, temporary home, direct provider call, or direct bridge test is diagnostic only.

- [x] **Step 4: Independently record executable provenance and stability**

```bash
core_pid=$(systemctl show aletheon-core.service -p MainPID --value)
user_pid=$(systemctl --user show aletheon.service -p MainPID --value)
core_before=$(systemctl show aletheon-core.service -p NRestarts --value)
user_before=$(systemctl --user show aletheon.service -p NRestarts --value)
sha256sum target/release/aletheon /usr/bin/aletheon "/proc/${core_pid}/exe" "/proc/${user_pid}/exe"
sleep 10
core_after=$(systemctl show aletheon-core.service -p NRestarts --value)
user_after=$(systemctl --user show aletheon.service -p NRestarts --value)
test "${core_before}" = "${core_after}"
test "${user_before}" = "${user_after}"
systemctl is-active --quiet aletheon-core.service
systemctl --user is-active --quiet aletheon.service
```

Expected: all four SHA-256 values are identical; both services remain active; both restart counters remain unchanged. Record the common digest, PIDs, counters, deployment receipt path, and governed-request receipt path in the implementation handoff.

- [x] **Step 5: Inspect the complete implementation diff**

```bash
git status --short
git diff --stat origin/dev...HEAD
git diff --check origin/dev...HEAD
```

Expected: the diff is limited to specialized profile assets, role resolution, workspace attenuation, typed finding scope, their tests, and generated acceptance receipts owned by the deployment workflow. No unrelated crate, state authority, prompt-specific runtime behavior, or audit input document is changed.

- [x] **Step 6: Preserve deployment evidence without inventing tracked files**

```bash
git status --short
```

Expected: deployment creates no tracked repository changes. Reference the external receipt locations printed by `scripts/aletheon.sh deploy` in the implementation handoff and create no evidence-only file. Any unexpected tracked change fails this step and must be investigated before completion.

## Validation evidence — 2026-08-02

- Bundled profile inventory, strict resolver contracts, Fabric cognitive
  contracts, workspace authority, scoped filesystem, AgentControl admission,
  Native Cognit runtime, and cognitive workflow tests: PASS through
  `scripts/cargo-agent.sh`.
- `specialized_role_capabilities`: PASS in three consecutive deterministic
  runs. Each run observed an authoritative AgentControl terminal snapshot and
  checked final file bytes for Planner, Explorer, Reviewer, Tester, Executor,
  and Fixer boundaries.
- `bash scripts/cargo-agent.sh fmt --all -- --check`: PASS.
- `git diff --check`: PASS for the complete implementation range.
- `sudo bash scripts/aletheon.sh deploy`: PASS. The deploy gate seeded all six
  shipped profiles, reported the profile registry ready, completed the official
  Memory Agent smoke, and completed a real request through the installed client
  and official user socket.
- Release, `/usr/bin/aletheon`, machine daemon, and user daemon SHA-256:
  `822d411f57116b114c281ca0e125c637ee92249ec5153a9e513e7a364928a002`.
- Independent stability observation: machine PID `1739781`, user PID `1739800`,
  both services active, and both `NRestarts` values remained `0` across the
  observation interval.
- The two external audit input documents remain untracked and unchanged.

## Completion audit

| Acceptance item | Implemented by | Proof |
|---|---|---|
| Six distinct effective profiles | Tasks 1-2 | Bundled inventory plus exact resolver map |
| No `code-agent` fallback | Task 2 | Bootstrap resolver negative tests and deleted fallback branch |
| Planner/Reviewer cannot mutate | Tasks 2, 4, 6 | Exact tool set, empty write authority, final bytes |
| Tester validates without source writes | Tasks 2, 4, 6 | `validation_run` succeeds while `file_write` fails |
| Executor is task-scoped and cannot Review | Tasks 4-6 | Narrowed policy and typed artifact rejection |
| Fixer is finding-scoped | Tasks 3-6 | Typed paths, admitted path union, sibling unchanged |
| Invalid input fails closed | Tasks 2-6 | Missing profile, extra tool, traversal, symlink, empty scope, forged artifact tests |
| Terminal receipt and final files verified | Task 6 | Three deterministic runs using `AgentControlPort::wait` |
| Focused checks and formatting pass | Task 7 | Exact command sequence and `git diff --check` |
| Installed deployment passes | Task 7 | Deploy receipt, equal digests, stable counters, real official-socket request |

## Scope guardrails

- This plan implements only SUB-P0-1; do not combine other audit workstreams in the same implementation series.
- Do not create a new top-level crate or a new task, memory, plan, receipt, or Agent state authority.
- Do not encode any production decision using a test prompt, natural-language phrase, language, repository name, fixed checkout path, or expected answer.
- Keep model output explanatory only; authority derives from typed role, effective tool registry, trusted workspace, binding scope, finding paths, and Host validation.
- Report provider inference rounds, provider retries, tool calls, active context, and cumulative usage separately during any real-runtime investigation.
- Preserve the two untracked audit documents as external requirement inputs; do not stage, modify, rename, or delete them.
