# Aletheon M6 Google Read-Only Detailed Plan

> **For agentic workers:** Execute one task at a time, test first, and stop after each commit with the staged diff and command evidence.

**Goal:** Bind one or more Google accounts securely and expose Gmail search/read plus Calendar list operations through Aletheon's normal capability path without exposing credentials to Cognit.

**Architecture:** Extend the existing OAuth implementation rather than adding a second auth stack. Split token persistence behind an interface, encrypt credentials at rest, persist account/grant metadata in the executive database, and register native Google capability tools in `ToolRegistry`. Native and MCP-backed tools share the same admission path; Google REST credentials remain inside Corpus adapters.

**Tech Stack:** Rust, Tokio, reqwest async client, SQLite, AES-256-GCM, OAuth 2.0 Authorization Code + PKCE S256, Gmail API, Calendar API, existing Tool/ToolRegistry and Telegram channel.

---

## 1. Requirement and code anchors

- Google must sit behind Executive permission/approval checks and Cognit must not receive tokens: `docs/arch/agent-google/02_GOOGLE_ECOSYSTEM_INTEGRATION.md:6-24`.
- Identity, grants, multiple accounts, revocation, and incremental authorization are required: `docs/arch/agent-google/02_GOOGLE_ECOSYSTEM_INTEGRATION.md:40-74`.
- M6 is read-only Gmail/Calendar with binding, manual refresh, and Telegram queries: `docs/arch/agent-google/05_IMPLEMENTATION_ROADMAP.md:128-138`.
- Current `TokenStore` reads and writes plaintext JSON: `crates/corpus/src/tools/mcp/auth.rs:141-205`.
- Current OAuth exchange/refresh uses blocking HTTP: `crates/corpus/src/tools/mcp/auth.rs:361-435`.
- `McpServerConfig` currently has no auth configuration: `crates/corpus/src/tools/mcp/config.rs:22-35`.
- The existing registry accepts native `Tool` implementations: `crates/corpus/src/tools/tools/registry.rs:9-59`.

**Scope decision:** M6 registers native Google tools alongside MCP wrappers. It does not modify `McpServerConfig`, because Google is a first-party external provider with account binding and synchronization needs beyond a generic server transport. Both paths still converge on `ToolRegistry` and Executive admission.

## 2. Task 1 — Define external identity and read-only contracts

**Files:**

- Create: `crates/fabric/src/types/external_identity.rs`
- Create: `crates/fabric/src/types/google.rs`
- Modify: `crates/fabric/src/types/mod.rs`
- Modify: `crates/fabric/src/lib.rs`

- [ ] First add serde/validation tests for `ExternalIdentityId`, `IdentityProvider::Google`, `ExternalIdentity`, `ExternalScope`, `CapabilityGrant`, and revocation state.
- [ ] Define bounded Gmail query/message-summary/message DTOs and Calendar time-range/event DTOs; exclude access/refresh tokens from every fabric type.
- [ ] Every imported record carries provider account ID, provider object ID, fetched-at time, source timestamp, and optional ETag/history marker.
- [ ] Model read and write scopes separately; M6 accepts only `gmail.readonly`, `calendar.readonly`, and identity scopes needed to bind the account.
- [ ] Test redacted Debug/Display output and reject oversized query/page-size/time-range inputs.
- [ ] Run `cargo test -p fabric -- types::external_identity types::google`; expect PASS.
- [ ] Commit `feat(fabric): define external identity contracts`.

## 3. Task 2 — Introduce token persistence boundary

**Files:**

- Modify: `crates/corpus/src/tools/mcp/auth.rs`
- Create: `crates/corpus/src/tools/mcp/token_store.rs`
- Modify: `crates/corpus/src/tools/mcp/mod.rs`

- [ ] Write contract tests for load/get/set/remove/atomic-save and multi-account keys before moving storage code.
- [ ] Define `TokenPersistence: Send + Sync` with whole-entry read/write/delete operations; key entries by provider plus external identity, not only MCP server name.
- [ ] Keep `TokenEntry` private to the credential layer or ensure Debug is fully redacted.
- [ ] Retain `TokenStore::new/open_default/get/set/remove/save` as a compatibility facade backed by the trait so existing MCP tests and callers remain valid.
- [ ] Do not silently read legacy plaintext into the encrypted store; expose an explicit one-shot migration operation that deletes plaintext only after encrypted write and reread succeed.
- [ ] Run `cargo test -p corpus -- tools::mcp::auth tools::mcp::token_store`; expect PASS.
- [ ] Commit `refactor(corpus): isolate oauth token persistence`.

## 4. Task 3 — Encrypt token storage atomically

**Files:**

- Create: `crates/corpus/src/security/credential_vault.rs`
- Modify: `crates/corpus/src/security/mod.rs`
- Modify: `crates/corpus/Cargo.toml`
- Modify: workspace dependency manifest if dependencies are centralized
- Modify: `crates/fabric/src/paths.rs`

- [ ] Add failing tests proving ciphertext contains neither access token, refresh token, account email, nor serialized token JSON.
- [ ] Implement AES-256-GCM with a fresh 96-bit CSPRNG nonce on every save; authenticate a fixed magic string plus format version as AAD.
- [ ] Use an explicit versioned envelope containing only magic, version, nonce, and ciphertext. Reject unknown versions, wrong keys, truncated data, and modified ciphertext.
- [ ] Load the 32-byte master key from a configured root-owned secret file; never accept it in command arguments, config serialization, logs, or model-visible tool results.
- [ ] Require Unix mode `0600` for key and vault files, create through `create_new`, write a sibling temporary file, `sync_all`, rename atomically, then sync the parent directory.
- [ ] Make non-Unix behavior fail closed until equivalent ACL checks exist.
- [ ] Add explicit legacy migration tests: plaintext remains on failed migration; successful migration is reread before best-effort secure deletion and emits no token values.
- [ ] Run `cargo test -p corpus -- security::credential_vault`; expect PASS.
- [ ] Commit `feat(corpus): encrypt oauth credentials at rest`.

## 5. Task 4 — Make OAuth asynchronous and PKCE-bound

**Files:**

- Modify: `crates/corpus/src/tools/mcp/auth.rs`
- Create: `crates/corpus/src/tools/google/oauth.rs`
- Create: `crates/corpus/src/tools/google/mod.rs`

- [ ] Add HTTP-mock tests for authorization URL, CSRF expiry/replay, PKCE verifier mismatch, code exchange, refresh-token preservation, scope reduction, revocation, and redacted errors.
- [ ] Extract a reusable async OAuth client using `reqwest::Client`; remove blocking HTTP from daemon execution paths.
- [ ] Generate PKCE S256 verifier/challenge per authorization attempt and bind it to the single-use CSRF state with a ten-minute lifetime.
- [ ] Google configuration uses official authorization/token/revocation endpoints, `access_type=offline`, and `include_granted_scopes=true`; do not request write scopes in M6.
- [ ] Preserve a previously issued refresh token when a refresh response omits it; reject a returned grant whose effective scopes exceed the requested set.
- [ ] Keep tokens inside the provider and return only authenticated HTTP responses or normalized DTOs.
- [ ] Run `cargo test -p corpus -- tools::google::oauth tools::mcp::auth`; expect PASS.
- [ ] Commit `feat(corpus): add async pkce oauth flow`.

## 6. Task 5 — Persist account binding and grants

**Files:**

- Modify: `crates/executive/src/impl/goal/migrations.rs`
- Create: `crates/executive/src/impl/external/mod.rs`
- Create: `crates/executive/src/impl/external/repository.rs`
- Modify: `crates/executive/src/impl/mod.rs`

- [ ] Add `external_identities`, `capability_grants`, and append-only `external_identity_events` tables; store provider subject/email/grant metadata but never tokens.
- [ ] Implement bind/get/list/revoke/update-grant with optimistic versions and unique `(provider, provider_subject, principal_id)` binding.
- [ ] Complete binding only after OAuth succeeds and Google's user-info subject is fetched; authenticated local/Telegram principal is authoritative, never request JSON.
- [ ] Revocation marks the grant inactive first, deletes vault credentials, and then best-effort calls provider revocation. Missing credentials remain revoked locally.
- [ ] Test multiple accounts, duplicate binding, cross-principal access, restart, partial revocation failure, and reduced scopes.
- [ ] Run `cargo test -p executive -- impl::external`; expect PASS.
- [ ] Commit `feat(executive): persist external account grants`.

## 7. Task 6 — Implement Gmail and Calendar read adapters

**Files:**

- Create: `crates/corpus/src/tools/google/client.rs`
- Create: `crates/corpus/src/tools/google/gmail.rs`
- Create: `crates/corpus/src/tools/google/calendar.rs`
- Modify: `crates/corpus/src/tools/google/mod.rs`
- Test: `crates/corpus/tests/google_read_only.rs`

- [ ] Define capability traits matching the read subset of `docs/arch/agent-google/02_GOOGLE_ECOSYSTEM_INTEGRATION.md:76-121`; leave create/send methods absent from M6 implementations.
- [ ] Gmail supports bounded search, important unread search, metadata/list, and explicit message read. Calendar supports bounded event listing with timezone and pagination.
- [ ] Use account-bound token refresh, finite connect/request timeouts, bounded pages/result counts/body bytes, retry-after handling, and cancellation.
- [ ] Normalize provider errors into stable categories without response bodies that may contain sensitive data.
- [ ] Reject an account not owned by the invoking principal and reject any effective write grant.
- [ ] Test pagination, 401 refresh-once, 403 scope denial, 429/backoff, malformed payload, cancellation, size limits, and account isolation.
- [ ] Run `cargo test -p corpus --test google_read_only`; expect PASS.
- [ ] Commit `feat(corpus): add read-only Google adapters`.

## 8. Task 7 — Register principal-aware Google tools

**Files:**

- Create: `crates/corpus/src/tools/google/tools.rs`
- Modify: `crates/corpus/src/tools/tools/registry.rs`
- Modify: `crates/executive/src/impl/daemon/handler/init.rs`
- Test: `crates/executive/tests/google_tool_flow.rs`

- [ ] Add tools `google_gmail_search`, `google_gmail_read`, and `google_calendar_list`; schemas expose account aliases/IDs and bounded query fields only.
- [ ] Resolve principal and account through trusted `ToolContext`/Executive services; never allow the model to submit a principal ID as authority.
- [ ] Register tools only when Google integration and the corresponding active read grant exist.
- [ ] Ensure tool results contain normalized data/provenance and no Authorization headers, token fields, raw HTTP dumps, or unbounded message bodies.
- [ ] Route calls through the same Executive permission, budget, audit, and cancellation path as existing tools.
- [ ] Test forged account IDs, absent/revoked grants, schema bounds, audit records, and token-redaction snapshots.
- [ ] Run `cargo test -p executive --test google_tool_flow`; expect PASS.
- [ ] Commit `feat(executive): register Google read tools`.

## 9. Task 8 — Add manual refresh and Telegram query flow

**Files:**

- Create: `crates/executive/src/impl/daemon/handler/rpc/rpc_google.rs`
- Modify: `crates/executive/src/impl/daemon/handler/rpc.rs`
- Modify: `crates/executive/src/impl/channel/telegram/mod.rs`
- Test: `crates/executive/tests/google_telegram_query.rs`

- [ ] Add authenticated local RPC for authorization start/callback, account list/revoke, and manual token refresh; responses redact credentials.
- [ ] Telegram natural-language queries use the normal ReAct/tool path for “today's events” and “important unread mail”; no provider-specific bypass around Cognit/Executive.
- [ ] If multiple accounts match, return a bounded account-choice prompt; never guess an account for a sensitive query.
- [ ] Token refresh is singleflight per account and manual refresh reports only success, required reauthorization, or a stable error code.
- [ ] Test account disambiguation, revoked account, expired access token, refresh failure, unauthorized Telegram identity, restart, and no-token transcript snapshots.
- [ ] Run `cargo test -p executive --test google_telegram_query`; expect PASS.
- [ ] Commit `feat(executive): expose Google read-only workflows`.

## 10. M6 release audit

- [ ] Run `cargo fmt --all -- --check`.
- [ ] Run all scoped tests above, then `cargo test --workspace` and `cargo build --workspace`.
- [ ] Search vault files, SQLite, logs, Goal events, tool transcripts, and test snapshots; prove refresh/access tokens occur only in encrypted vault memory/ciphertext.
- [ ] Prove the exact authorized scopes are read-only and write/send/create operations are unavailable.
- [ ] Prove two principals and two Google accounts cannot cross-read.
- [ ] Prove daemon restart preserves binding and encrypted credentials and manual refresh still works.
- [ ] Prove current MCP bearer/OAuth tests remain green after the persistence/async refactor.
- [ ] Manually demonstrate Telegram answers for today's events and important unread mail with source account and timestamps.

## 11. DeepSeek batches

1. Tasks 1–3: contracts, persistence boundary, encrypted vault.
2. Tasks 4–5: async OAuth and account repository.
3. Tasks 6–7: adapters and tools.
4. Tasks 8–10: Telegram/RPC and audit.

Guardrails:

```text
Do not create a parallel OAuth stack.
Do not store or log plaintext tokens.
Do not request Google write scopes in M6.
Do not trust principal/account identity from model arguments.
Do not use blocking HTTP in async daemon paths.
Stop after each batch with exact tests and staged diff.
```
