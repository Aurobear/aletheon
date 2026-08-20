# Agent Kernel V2 foundation checkpoint

Date: 2026-08-09  
Baseline: `dev@bd1ceac2965832f15cf45b811fd34e052e7e9a67`  
Implementation head inspected: PR #192 `80358c975f15252139f0983dd3709cfe00e8fd51`  
State: foundation checkpoint complete; `RA-00` remains the next canonical slice

## Requirement anchors

- The first post-PR-#192 task is to freeze the architecture ratchet and verify that the deletion did not remove a known external public consumer.
- Wave A may remove only caller-zero ghost authorities and must not cut a canonical writer.
- The retired TUI Session authority, dormant Executive Agent authority, and duplicate loader are explicitly classified for deletion.
- `RA-00` must separately freeze the complete constructor/ID/writer/table/composition census; this checkpoint does not claim that exit condition.

## Context receipt

```text
Slice: foundation checkpoint before RA-00
Baseline commit: bd1ceac2965832f15cf45b811fd34e052e7e9a67
Direct prerequisites: PR #192 code foundation at 80358c975f15252139f0983dd3709cfe00e8fd51
Current authoritative writer: unchanged legacy Executive Session/Turn/AgentControl paths
Target owner/writer: unchanged by this checkpoint
IDs minted here: none
Production callers: zero workspace callers of the three retired authorities; surviving profile loader is called from daemon bootstrap
Installed/config callers: none identified for retired authorities; surviving profile loader consumes installed profile paths
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none added
Deletion owner: RA-00 foundation cleanup already represented by PR #192
Unknowns/blockers: full constructor/ID/writer/registry census remains RA-00
Expected files: retired-authority inventory, architecture gate, this evidence record
Out-of-scope files: Runtime/Kernel/Application/Gateway/TUI production behavior and all writer cutovers
```

## Deletion evidence

The repository baseline contained only definitions, local tests, and public re-exports for the deleted authorities. The PR #192 head has no matching workspace production call sites:

```text
git grep -n -E '\b(TuiSessionManager|application::agent::AgentRuntime|composition::agents)\b' HEAD -- 'crates/**/*.rs'
# no matches for the retired paths/symbols

git grep -n -E '\b(TuiSessionManager|AgentRuntime)\b' bd1ceac2 -- 'crates/**/*.rs'
# AgentRuntime definition/re-export confined to executive application/agent and executive/lib.rs
# TuiSessionManager definition/re-export confined to executive core/session.rs and executive/core/mod.rs
```

The distinct production loader remains at `crates/adapters/agent-profile/src/lib.rs:49` and is composed by `crates/aletheon/src/wiring/daemon/bootstrap/runtime.rs:11-120`. PR #192 removed only `composition/agents`, whose callers were its own unit tests.

## External public-consumer check

Checks run on 2026-08-09:

```text
GET https://crates.io/api/v1/crates/aletheon  -> 404
GET https://crates.io/api/v1/crates/executive -> 404

gh api --method GET search/code -f q='"executive::core::TuiSessionManager"'       -> total_count 0
gh api --method GET search/code -f q='"executive::runtime::AgentRuntime"'         -> total_count 0
gh api --method GET search/code -f q='"executive::application::agent::AgentRuntime"' -> total_count 0
gh api --method GET search/code -f q='"executive::composition::agents"'            -> total_count 0
```

This proves there is no published crates.io package and no indexed public GitHub use of the exact retired paths. It cannot prove the absence of private or non-indexed git consumers. The removal remains a Rust API break for such a consumer, but no known supported external contract was found.

## Ratchet hardening

`config/architecture/retired-authorities.tsv` is now the reviewable source for monotonic path/symbol retirement. The gate rejects:

- files, directories, symlinks, and broken symlinks at retired paths;
- a reintroduced `TuiSessionManager` declaration/re-export anywhere under `crates`;
- a reintroduced `AgentRuntime` declaration/re-export under Executive Application;
- incomplete rows, unknown row kinds, and invalid Rust identifiers.

The canonical name remains available to the future `runtime` owner; the gate does not ban `runtime::AgentRuntime`.

## Validation

```text
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture
# PASS: 23 findings, 0 dependencies, 4 paths; no additions

bash scripts/cargo-agent.sh check -p executive --lib
# PASS

bash scripts/cargo-agent.sh fmt --all -- --check
# PASS

bash scripts/cargo-agent.sh test -p executive --lib composition::agent_loader -- --nocapture
# PASS: 11 passed; 0 failed; 771 filtered out

# Local mutation probes, removed after each assertion:
# broken retired symlink       -> rejected
# empty retired authority root -> rejected
# private AgentRuntime alias   -> rejected
# cross-crate TuiSessionManager declaration -> rejected
```

No writer, schema, installed runtime, daemon, socket, or client behavior changed, so this checkpoint is not deployment acceptance and does not require a system deploy. The next implementation unit is evidence-only `RA-00`.
