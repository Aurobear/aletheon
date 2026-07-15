# M01 Memory Contract Baseline Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Freeze the actual local-first memory write, reopen, recall-bound and supplemental-outage behavior before changing memory semantics.

**Architecture:** Exercise `DefaultMemoryService` through its public `MemoryService` facade against real temporary SQLite files. Use a failing supplemental test double only through `CompositeMemoryService`; baseline tests inspect durable backend state without pretending that message and reflection recall already works.

**Tech Stack:** Rust, Tokio, rusqlite-backed Mnemosyne stores, `tempfile`, Cargo integration tests.

**Prerequisites:** S02 complete.

**Unlocks:** M02 canonical memory records and scopes.

---

## Requirement and code anchors

- Durable user/assistant messages, reflection/decision/outcome recording, bounded projection and supplemental outage behavior are required by `docs/plans/2026-07-15-mnemosyne-unified-memory-plan.md:302-326`.
- `DefaultMemoryService::record` writes messages to `RecallMemory` and the other three event kinds to `EpisodicMemory` at `crates/mnemosyne/src/service.rs:289-333`.
- `DefaultMemoryService::recall` currently queries only `FactStore` at `crates/mnemosyne/src/service.rs:336-400`; therefore message/reflection target tests must remain ignored until M03.
- `CompositeMemoryService::recall` keeps supplemental transport behind a timeout but currently fails if local recall fails at `crates/mnemosyne/src/composite_service.rs:169-207`.

## Invariants and non-goals

- Tests use public APIs and real SQLite reopen behavior.
- Default tests remain green; known M03 target behavior is represented by explicitly ignored tests.
- GBrain failure may degrade supplemental health but must not erase successful local recall.
- This slice does not introduce canonical record types, change live recall routing or implement forgetting.

## File map

- Create: `crates/mnemosyne/tests/unified_memory_contract.rs`
- Modify only if the contract exposes a real baseline defect: `crates/mnemosyne/src/service.rs`
- Modify only if outage behavior violates the source requirement: `crates/mnemosyne/src/composite_service.rs`

### Task 1: Build a real SQLite-backed test fixture

- [ ] **Step 1: Add a fixture that creates all four stores**

```rust
async fn open_service(root: &Path) -> DefaultMemoryService {
    let clock: Arc<dyn Clock> = Arc::new(TestClock::default());
    let recall = Arc::new(Mutex::new(RecallMemory::new(&root.join("recall.db"), clock.clone()).unwrap()));
    let facts = Arc::new(Mutex::new(FactStore::open(&root.join("facts.db")).unwrap()));
    let core = Arc::new(Mutex::new(CoreMemory::new()));
    let mut episodic = EpisodicMemory::new(root.join("episodic.db"), clock.clone());
    episodic.init(&SubsystemContext { name: "contract".into(), working_dir: root.into(), config: Value::Null, bus: None }).await.unwrap();
    DefaultMemoryService::new(recall, facts, core, Arc::new(Mutex::new(episodic)), clock)
}
```

- [ ] **Step 2: Compile the fixture**

Run: `cargo test -p mnemosyne --test unified_memory_contract --no-run`

Expected: PASS; the integration test binary is built.

### Task 2: Prove all experience variants survive reopen

- [ ] **Step 1: Add user and assistant message reopen tests**

Record distinct messages, drop the service, reopen it and assert `RecallMemory::search` returns each payload in its session.

- [ ] **Step 2: Add reflection, architecture-decision and goal-outcome reopen tests**

Record all three variants, reopen `EpisodicMemory`, call `recall_reflections(10)` and assert their stable record IDs and contents are present.

- [ ] **Step 3: Run persistence tests**

Run: `cargo test -p mnemosyne --test unified_memory_contract persists -- --nocapture`

Expected: PASS with five durable event records across the two SQLite stores.

### Task 3: Document the known local recall asymmetry as executable targets

- [ ] **Step 1: Add ignored message recall target**

```rust
#[tokio::test]
#[ignore = "known M03 gap: DefaultMemoryService recall only queries FactStore"]
async fn recorded_message_is_recalled_in_its_session() { /* real fixture and assertion */ }
```

- [ ] **Step 2: Add ignored reflection recall target**

```rust
#[tokio::test]
#[ignore = "known M03 gap: DefaultMemoryService recall does not query EpisodicMemory"]
async fn recorded_reflection_is_recalled_for_relevant_query() { /* real fixture and assertion */ }
```

- [ ] **Step 3: Verify ignored tests are discoverable and default-green**

Run: `cargo test -p mnemosyne --test unified_memory_contract -- --list`

Expected: both target test names are listed with `test` type.

### Task 4: Prove request bounds and supplemental outage isolation

- [ ] **Step 1: Add item and byte-bound tests using real facts**

Insert multiple matching facts and assert one final `RecallSet` never exceeds `max_items` or `max_content_bytes`.

- [ ] **Step 2: Add a supplemental service that returns degraded outage health**

```rust
struct OutageSupplemental;

#[async_trait]
impl SupplementalMemoryService for OutageSupplemental {
    fn queue_depth(&self) -> usize { 0 }
    fn record(&self, _: &ExperienceEvent, _: i64) -> Result<EnqueueOutcome, GbrainBackendError> { /* return transport error */ }
    async fn recall(&self, _: RecallRequest, _: &CancellationToken) -> SupplementalRecall { /* empty degraded result */ }
    fn forget(&self, _: ForgetPolicy) -> Result<(), GbrainBackendError> { Ok(()) }
}
```

- [ ] **Step 3: Assert local facts remain available and health is degraded**

Run: `cargo test -p mnemosyne --test unified_memory_contract supplemental_outage_keeps_local_recall`

Expected: PASS; the local fact is returned and `CompositeMemoryHealth.degraded` is true.

### Task 5: Record schema paths and migration baseline

- [ ] **Step 1: Assert the expected database files exist after initialization**

Assert `recall.db`, `facts.db` and `episodic.db` exist. Open each with rusqlite and assert its defining table exists (`recall_memory`, `facts`, `reflection_events`).

- [ ] **Step 2: Run the complete M01 suite**

Run: `cargo test -p mnemosyne --test unified_memory_contract`

Expected: all active tests PASS and exactly two M03 target tests are ignored.

### Task 6: Workspace verification and commit boundary

- [ ] **Step 1: Run scoped and workspace checks**

```bash
cargo fmt --all -- --check
cargo clippy -p mnemosyne --all-targets -- -D warnings
cargo test -p mnemosyne
cargo test --workspace
bash tests/architecture_check.sh
bash scripts/architecture-check.sh
```

Expected: every command exits 0; architecture findings do not grow.

- [ ] **Step 2: Inspect and commit only M01 files**

```bash
git diff --check
git add docs/plans/2026-07-16-original-plan-coverage-matrix.md \
        docs/plans/2026-07-16-m01-memory-contract-baseline.md \
        crates/mnemosyne/tests/unified_memory_contract.rs
git diff --cached
git commit -F- <<'MSG'
test(mnemosyne): establish unified memory baseline

The memory facade writes to several durable stores while its read path still
covers only facts, so later unification needs an executable behavior baseline.

- prove every experience variant survives SQLite reopen
- freeze request bounds and supplemental-outage isolation
- retain ignored target tests for the known M03 recall gaps
MSG
```

## Compatibility deletion gate

The two `#[ignore = "known M03 gap: ..."]` attributes must be removed in M03 when unified recall queries `RecallMemory` and `EpisodicMemory`. M01 is not considered transitively complete at V01 while either ignored target remains.

## Completion evidence

- [ ] all five experience variants have durable reopen evidence;
- [ ] local recall survives a supplemental outage;
- [ ] request item/byte bounds are enforced;
- [ ] database paths and defining tables are asserted;
- [ ] two known-gap targets are present and explicitly ignored;
- [ ] scoped, workspace and architecture checks pass.
