# Code TODOs

Status: **Open** | All linked to phased development tasks

---

## TODO 1 — io Driver Bindings (Phase 7/8)

- **File:** `crates/corpus/src/drivers/io/mod.rs:1`
- **Tag:** `// TODO: Phase 7/8 implementation`
- **Description:** Entire io driver module is unimplemented.

---

## TODO 2 — proc Driver Bindings (Phase 7/8)

- **File:** `crates/corpus/src/drivers/proc/mod.rs:1`
- **Tag:** `// TODO: Phase 7/8 implementation`
- **Description:** Entire proc driver module is unimplemented.

---

## TODO 3 — Wire ShellEscalationDetector into Runner (D1-T11)

- **File:** `crates/corpus/src/security/runner.rs:476`
- **Tag:** `// TODO(D1-T11): Insert ShellEscalationDetector scan here`
- **Description:** ShellEscalationDetector is implemented but not wired into the command execution runner. Shell commands are not scanned for escape attempts.

---

## TODO 4 — Migrate Dasein to WallTime

- **File:** `crates/dasein/src/core/continuity.rs:18`
- **Tag:** `// TODO: Migrate to WallTime`
- **Description:** Extensive `chrono::Duration` arithmetic and SQLite rfc3339 serialization need migration to the project's `WallTime` abstraction.

---

## TODO 5 — Spawn Exec-Server When Streaming Tools Enabled (D1-T9)

- **File:** `crates/executive/src/host/mod.rs:160`
- **Tag:** `// TODO(D1-T9): Spawn exec-server when grok_hardening.streaming_tools is enabled`
- **Description:** The exec-server process isn't spawned automatically; requires feature flag.

---

## TODO 6 — Call WorkspaceTrustResolver (D2-M3-T4)

- **File:** `crates/executive/src/impl/daemon/handler/mod.rs:171`
- **Tag:** `// TODO(D2-M3-T4): Call WorkspaceTrustResolver::evaluate() here`
- **Description:** Workspace trust resolution is not invoked during request handling.

---

## TODO 7 — Implement Full Tool Stream Bridging (D1-T10)

- **File:** `crates/executive/src/service/tool_stream_bridge.rs:43`
- **Tag:** `// TODO(D1-T10): Implement full bridging — drain receiver and emit`
- **Description:** The tool stream bridge between daemon and exec-server is a stub.

---

## TODO 8-9 — Migrate Fabric/Cognit to WallTime (2 sites)

- **File:** `crates/fabric/src/include/cognit.rs:107,231`
- **Tag:** `// TODO: Migrate to WallTime`
- **Description:** Multi-crate `chrono` formatting in `CognitSummary::summary()` and construction sites need migration to `WallTime`.

---

## TODO 10 — Implement Arrow RecordBatch in Vector Store

- **File:** `crates/mnemosyne/src/impl/vector_store.rs:238`
- **Tag:** `// TODO: Implement with Arrow RecordBatch`
- **Description:** The vector store has a stubbed method signature that should use Apache Arrow RecordBatch for efficient vector operations.
