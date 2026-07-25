# Phase 10 compatibility and security audit

Source requirements: `docs/arch/CORE_ARCHITECTURE_DECOUPLING_REFACTOR_PLAN.md:958-975` and `docs/plans/core-refactor/11_PHASE_10_GLOBAL_VERIFICATION.md:94-100`.

The authoritative execution is the green `bash scripts/cargo-agent.sh test --workspace` run in `08-workspace-test.log` (330.44 s). The following named tests are present as `ok` in that log.

| Requirement | Direct evidence | Result |
|---|---|---|
| Legacy configuration normalizes canonically | `layered_config_contract::legacy_channel_and_coding_keys_decode_to_canonical_owned_types`; `layered_config_contract::legacy_supplemental_memory_keys_are_read_but_never_reemitted`; `corpus::tools::mcp::config::tests::legacy_transport_and_oauth_schema_remain_compatible` | pass |
| Persistence migration is transactional, retryable, and idempotent | `credential_vault::failed_migration_keeps_plaintext_source`; `credential_vault::explicit_legacy_migration_verifies_then_securely_removes_plaintext`; `mcp::token_store::migration_preserves_source_on_failed_target_write`; `gbrain_spool::legacy_migration_redacts_commits_then_renames_and_restarts_idempotently`; `attempt_coordinator::settlement_failure_retries_terminal_attempt_without_runtime_reinvocation` | pass |
| Credentials and provider failures are redacted | `google::oauth::escalation_pkce_rejection_and_provider_bodies_are_redacted`; `google::oauth::revocation_is_async_and_redacts_rejected_tokens`; `mcp::auth::display_redacts_token`; `mcp::token_store::token_entry_debug_is_redacted`; provider timeout tests | pass |
| OAuth scopes and state fail closed | `google::oauth::write_scopes_are_rejected`; `effective_scope_must_be_subset`; `unknown_and_replayed_state_fail_before_network`; MCP CSRF and endpoint-grant tests | pass |
| Sandbox and network policy fail closed | Corpus runner/sandbox tests, `platform::contract_suite::unavailable_sandbox_apply_fails_closed`, `executive::sandbox_first_fail_closed` and execd unrepresentable-policy tests | pass |
| Workspace trust fails closed | `workspace_trust::corrupt_file_store_fails_closed`; `authenticated_grant_rejects_undiscovered_source`; `headless_untrusted_repo_blocks_loader_but_keeps_normal_files_usable`; hook repository-trust tests | pass |
| Lease and emergency stop fail closed | Hardware simulator `permit_lease_deadline_and_sequence_fail_closed`, emergency-stop integration tests, Executive budget/quota lease tests | pass |
| Optional absence degrades; invalid configured integration fails | Supplemental-memory bootstrap/contract tests and MCP lifecycle/manager degraded/recovery tests; invalid integration/config tests reject construction | pass |
| Protocol/schema versions are explicit | External event v1/v2 compatibility and unknown-version rejection tests; client/execd protocol tests; HIL schema integrity tests; MCP initialize negotiation tests | pass |

Compatibility decisions:

- Persisted ExternalEvent v1 aliases remain read-only compatibility while writers emit v2 (`crates/fabric/src/types/external_event.rs:11-16`, `config/architecture/compatibility-debt.tsv:3-4`). Deleting them without evidence that supported v1 rows are gone would violate the data-preservation rule at `CORE_ARCHITECTURE_DECOUPLING_REFACTOR_PLAN.md:960-964`.
- Private adapter implementation names and private supplemental-memory SQLite table names are not core compatibility exceptions. Their obsolete ledger rows were removed; their placement is governed by the adapter boundary (`CORE_ARCHITECTURE_DECOUPLING_REFACTOR_PLAN.md:992-1001`).
- Unknown/newer versions remain explicit rejection paths; no silent provider/runtime fallback is permitted (`CORE_ARCHITECTURE_DECOUPLING_REFACTOR_PLAN.md:968-975`).
