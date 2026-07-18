# Deprecated APIs

Status: **Open** | Priority: Low — all still compile, migration path documented

---

## 1. Dasein::quick_mood_update()

- **File:** `crates/dasein/src/dasein/mod.rs` (~L74)
- **Tag:** `#[deprecated]`
- **Migration:** Use `record_outcome()` with explicit `OutcomeStatus` instead.
- **Why deprecated:** Legacy keyword-based mood adapter; too coarse-grained.

---

## 2. Agora::Workspace::commit()

- **File:** `crates/agora/src/workspace/mod.rs` (~L195)
- **Tag:** `#[deprecated]`
- **Migration:** Use `prepare_commit()` with `WorkspaceCommitPermit` instead.
- **Why deprecated:** No commit validation; new API enforces permission checks.

---

## 3. IpcBackend Trait

- **File:** `crates/fabric/src/ipc/ipc_types.rs` (~L11)
- **Tag:** `#[deprecated]` (trait-level)
- **Migration:** Use the `Transport` trait instead.
- **Why deprecated:** `IpcBackend` mixed transport and serialization concerns.

---

## 4. LLM Types: model_info()

- **File:** `crates/fabric/src/types/llm_types.rs` (~L60)
- **Tag:** `#[deprecated]`
- **Migration:** Use `name()` and `max_context_length()` directly.
- **Why deprecated:** Monolithic getter; split into focused accessors.

---

## 5. Mailbox::request_response()

- **File:** `crates/fabric/src/ipc/mailbox.rs` (~L12)
- **Tag:** `#[deprecated]`
- **Migration:** Use `MailboxService::request()` instead.
- **Why deprecated:** Renamed for consistency with the service pattern.
