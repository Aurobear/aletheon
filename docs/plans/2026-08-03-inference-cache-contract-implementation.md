# Inference Cache Contract Implementation Plan

> **For agentic workers:** Use `flow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every model request use deterministic tool schemas, one cache-aware usage contract, correct Anthropic prefix mapping, and linked terminal inference receipts.

**Architecture:** Fabric owns canonical inference types and deterministic schema hashing; Corpus returns stable catalogs; Cognit canonicalizes at the final provider boundary and maps native usage; Executive persists projection and terminal receipts without inferring unavailable cache values. Existing Session, Attempt, ContextAssembler, and provider caches remain the only relevant authorities.

**Tech Stack:** Rust, serde/schemars, SHA-256, async streams, existing Fabric/Cognit/Corpus/Executive contracts, system-installed Aletheon acceptance.

**Approved spec:** `docs/plans/2026-08-03-inference-cache-contract-design.md`

---

## Requirement traceability

| Requirement | Spec anchor | Implemented by |
|---|---|---|
| Canonical tool order/JSON/digest | design §4 | Tasks 1-2 |
| Unified optional cache usage | design §5 | Tasks 3-4 |
| Anthropic system and breakpoint mapping | design §6 | Task 5 |
| Linked terminal inference receipt | design §7 | Tasks 6-7 |
| Focused and installed acceptance | design §10 | Task 8 |

## File map

**Create:**

- `crates/fabric/src/types/inference_receipt.rs` — terminal inference receipt and status.
- `crates/fabric/tests/inference_contract.rs` — canonicalization, usage migration, and receipt schema contracts.
- `crates/cognit/tests/anthropic_cache_contract.rs` — provider request and streaming/non-stream usage fixture.
- `crates/cognit/tests/inference_recording.rs` — projection/terminal receipt linkage and exact-once behavior.

**Modify:**

- `crates/fabric/src/types/llm_types.rs` — canonical ToolDefinition functions and `InferenceUsage`.
- `crates/fabric/src/types/mod.rs`, `crates/fabric/src/lib.rs` — exports.
- `crates/fabric/src/types/model_projection.rs` — digest fields on pre-call projection.
- `crates/fabric/src/include/turn.rs` — inference receipt recording port.
- `crates/fabric/src/types/session.rs` — terminal inference item payload.
- `crates/fabric/src/ipc/stream.rs`, `crates/fabric/src/events/ui_event.rs` — canonical usage event.
- `crates/corpus/src/tools/tools/registry.rs` — stable registry snapshots.
- `crates/cognit/src/adapters/inference/{anthropic,openai_provider,ollama}.rs` — native usage mapping and final canonicalization.
- `crates/cognit/src/harness/{event_sink,session}.rs` — canonical usage propagation and inference recording.
- `crates/cognit/src/harness/linear/{step,tool_exec}.rs` — stream usage consumption.
- `crates/executive/src/composition/turn_service.rs` — persist terminal receipt item.
- `crates/executive/src/application/turn_pipeline.rs` and daemon format paths — aggregate optional cache dimensions.
- `crates/interact/src/tui/cli.rs` and reducer/response matches — display the canonical vocabulary.
- Provider mocks and focused tests found by the exact migration grep in Task 4.
- `config/architecture/executive-layers.tsv` only if a new Executive production file is added; this plan does not add one.

---

### Task 1: Canonical ToolDefinition contract

**Files:**
- Modify: `crates/fabric/src/types/llm_types.rs`
- Modify: `crates/fabric/src/lib.rs`
- Create: `crates/fabric/tests/inference_contract.rs`

- [ ] **Step 1: Add failing canonicalization tests**

Add tests that construct the same two tools in opposite insertion order and with
opposite nested JSON object insertion order:

```rust
#[test]
fn tool_schema_digest_is_order_and_object_key_independent() {
    let left = vec![tool("zeta", json!({"type":"object","properties":{"b":{"type":"string"},"a":{"type":"integer"}}})), tool("alpha", json!({}))];
    let right = vec![tool("alpha", json!({})), tool("zeta", json!({"properties":{"a":{"type":"integer"},"b":{"type":"string"}},"type":"object"}))];
    assert_eq!(canonicalize_tool_definitions(&left).unwrap(), canonicalize_tool_definitions(&right).unwrap());
    assert_eq!(tool_schema_digest(&left).unwrap(), tool_schema_digest(&right).unwrap());
}

#[test]
fn array_order_changes_digest_and_duplicate_names_fail() {
    let first = vec![tool("ordered", json!({"enum":["a","b"]}))];
    let second = vec![tool("ordered", json!({"enum":["b","a"]}))];
    assert_ne!(tool_schema_digest(&first).unwrap(), tool_schema_digest(&second).unwrap());
    assert!(canonicalize_tool_definitions(&[tool("dup", json!({})), tool("dup", json!({}))]).is_err());
}
```

- [ ] **Step 2: Run the Fabric test and observe missing symbols**

```bash
bash scripts/cargo-agent.sh test -p fabric --test inference_contract
```

Expected: FAIL because the canonicalizer and digest do not exist.

- [ ] **Step 3: Implement the canonicalizer and digest**

Add a typed error and these public functions beside `ToolDefinition`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ToolDefinitionCanonicalizationError {
    #[error("tool name is empty or contains control characters")]
    InvalidName,
    #[error("duplicate tool name: {0}")]
    DuplicateName(String),
    #[error("canonical tool schema serialization failed: {0}")]
    Serialization(String),
}

pub fn canonicalize_tool_definitions(
    definitions: &[ToolDefinition],
) -> Result<Vec<ToolDefinition>, ToolDefinitionCanonicalizationError> {
    let mut canonical = definitions.to_vec();
    for definition in &mut canonical {
        if definition.name.trim().is_empty() || definition.name.chars().any(char::is_control) {
            return Err(ToolDefinitionCanonicalizationError::InvalidName);
        }
        definition.input_schema = canonical_json(&definition.input_schema);
    }
    canonical.sort_by(|left, right| left.name.cmp(&right.name));
    for pair in canonical.windows(2) {
        if pair[0].name == pair[1].name {
            return Err(ToolDefinitionCanonicalizationError::DuplicateName(pair[0].name.clone()));
        }
    }
    Ok(canonical)
}
```

`canonical_json` recursively rebuilds objects through a `BTreeMap`, recursively
canonicalizes array elements without sorting the array, and clones scalars.
`tool_schema_digest` hashes domain-separated canonical JSON bytes using
`sha2::Sha256` and returns `sha256:<hex>`.

- [ ] **Step 4: Run the Fabric test**

```bash
bash scripts/cargo-agent.sh test -p fabric --test inference_contract
```

Expected: PASS.

- [ ] **Step 5: Commit canonical inference schemas**

Stage only the files in this task and commit with a full `feat(inference)` message.

---

### Task 2: Stable catalogs and final-boundary enforcement

**Files:**
- Modify: `crates/corpus/src/tools/tools/registry.rs`
- Modify: `crates/corpus/src/tools/capability_executor.rs`
- Test: `crates/corpus/src/tools/tools/registry.rs`
- Test: `crates/corpus/tests/capability_executor.rs`
- Test: `crates/executive/src/host/daemon/bootstrap/runtime/runtime_tests.rs`

- [ ] **Step 1: Add failing registry-order tests**

Register `zeta`, `alpha`, and `middle` in that order and assert both
`definitions()` and `profile_definitions()` return `alpha,middle,zeta`. Add a
catalog test that merges AgentControl definitions and asserts canonical ordering
and unique names.

- [ ] **Step 2: Run the narrow tests**

```bash
bash scripts/cargo-agent.sh test -p corpus tools::tools::registry::tests --lib
bash scripts/cargo-agent.sh test -p corpus --test capability_executor
```

Expected: the registry order assertion fails.

- [ ] **Step 3: Sort both registry snapshots and remove local sorting copies**

Build the vectors as today, then sort by exact name before returning. Keep the
final Fabric canonicalizer at provider dispatch; registry sorting is deterministic
discovery, not the security boundary. Replace `discover_tool_extensions`' local
sort with the already ordered snapshot and retain its deterministic test.

- [ ] **Step 4: Add a three-construction digest contract**

Build the same effective native/Goal tool set three times with different registry
insertion order and assert `tool_schema_digest` is identical. The test must call
the real profile merge path rather than sorting its expected vector.

- [ ] **Step 5: Run focused Corpus and bootstrap tests**

```bash
bash scripts/cargo-agent.sh test -p corpus tools::tools::registry::tests --lib
bash scripts/cargo-agent.sh test -p corpus --test capability_executor
bash scripts/cargo-agent.sh test -p executive host::daemon::bootstrap::runtime --lib
```

Expected: PASS.

- [ ] **Step 6: Commit catalog determinism**

Commit the scoped files with a full `fix(tools)` message.

---

### Task 3: Unified InferenceUsage provider contract

**Files:**
- Modify: `crates/fabric/src/types/llm_types.rs`
- Modify: `crates/fabric/src/types/attempt.rs`
- Modify: `crates/cognit/src/adapters/inference/openai_provider.rs`
- Modify: `crates/cognit/src/adapters/inference/ollama.rs`
- Modify: `crates/cognit/src/adapters/inference/anthropic.rs`
- Test: `crates/fabric/tests/inference_contract.rs`
- Test: provider unit tests in the three adapter files

- [ ] **Step 1: Add failing usage semantic tests**

```rust
#[test]
fn legacy_usage_aliases_do_not_invent_cache_writes() {
    let usage: InferenceUsage = serde_json::from_value(json!({
        "tokens_in": 10,
        "tokens_out": 2,
        "cache_hit_tokens": 4,
        "cache_miss_tokens": 6
    })).unwrap();
    assert_eq!(usage.total_input_tokens, Some(10));
    assert_eq!(usage.output_tokens, Some(2));
    assert_eq!(usage.cache_read_tokens, Some(4));
    assert_eq!(usage.cache_write_tokens, None);
    assert_eq!(usage.cache_telemetry, CacheTelemetry::Unknown);
}
```

Add mapping tests proving Anthropic total is checked
`uncached + read + write`, OpenAI derives uncached only when cached tokens are
present and not greater than total, and Ollama reports cache telemetry as
Unsupported.

- [ ] **Step 2: Introduce the canonical type**

Implement the exact `InferenceUsage` and `CacheTelemetry` schema from design §5.
Provide checked constructors:

```rust
pub fn anthropic(uncached: u64, output: u64, read: Option<u64>, write: Option<u64>) -> Result<Self, String>;
pub fn openai(total: u64, output: u64, read: Option<u64>) -> Result<Self, String>;
pub fn unsupported(total: Option<u64>, output: Option<u64>) -> Self;
```

The old `Usage` type and `LlmResponse.cache_hit_tokens/cache_miss_tokens` fields
are removed. `LlmResponse` owns only `usage: InferenceUsage`.

- [ ] **Step 3: Map non-stream provider responses**

- Anthropic uses its four native fields and checked total.
- OpenAI uses prompt/completion and optional cached tokens.
- Ollama uses `InferenceUsage::unsupported`.

Missing cache fields are not converted to zero.

- [ ] **Step 4: Run provider and Fabric tests**

```bash
bash scripts/cargo-agent.sh test -p fabric --test inference_contract
bash scripts/cargo-agent.sh test -p cognit adapters::inference --lib
```

Expected: PASS after all constructors in those targets use the canonical type.

- [ ] **Step 5: Commit the unified provider response contract**

Commit with a full `feat(inference)` message.

---

### Task 4: Streaming, Turn events, clients, and Attempt aggregation

**Files:**
- Modify: `crates/fabric/src/types/llm_types.rs`
- Modify: `crates/fabric/src/ipc/stream.rs`
- Modify: `crates/fabric/src/events/ui_event.rs`
- Modify: `crates/cognit/src/harness/event_sink.rs`
- Modify: `crates/cognit/src/harness/linear/step.rs`
- Modify: `crates/cognit/src/harness/linear/tool_exec.rs`
- Modify: `crates/executive/src/application/turn_pipeline.rs`
- Modify: `crates/executive/src/host/daemon/handler/format.rs`
- Modify: `crates/interact/src/tui/cli.rs`
- Modify: every mock/test returned by:
  `rg -l 'cache_hit_tokens|cache_miss_tokens|StreamChunk::Usage|LlmResponse \{' crates --glob '*.rs'`

- [ ] **Step 1: Change streaming and event variants to one type**

Use this shape at every internal layer:

```rust
StreamChunk::Usage { usage: InferenceUsage }
Event::Usage { usage: InferenceUsage }
TurnEventV1::Usage { #[serde(flatten)] usage: InferenceUsage }
ClientEvent::Usage { #[serde(flatten)] usage: InferenceUsage }
```

`InferenceUsage` fields use serde aliases for historical `tokens_in`,
`tokens_out`, and `cache_hit_tokens`; unknown `cache_miss_tokens` is ignored.
New serialization emits only canonical names.

- [ ] **Step 2: Preserve streaming usage**

`tool_exec.rs` forwards the received usage unchanged and never constructs zero
cache fields. Anthropic and OpenAI streaming adapters populate the same mapping
helpers used by non-stream completion. Ollama emits Unsupported.

- [ ] **Step 3: Make aggregation completeness-aware**

Replace scalar cache accumulators with:

```rust
#[derive(Default)]
struct OptionalUsageTotal {
    sum: u64,
    complete: bool,
    seen: bool,
}
```

Initialize `complete=true`; observing `Some(value)` marks seen and adds; observing
`None` sets complete=false. The projected aggregate is `Some(sum)` only when
`seen && complete`, otherwise `None`. Total input/output use the same rule where
persisted metrics accept optional observations.

- [ ] **Step 4: Update clients and mocks mechanically but semantically**

For every grep result, replace legacy literals with one of:

```rust
InferenceUsage::reported(...)
InferenceUsage::unsupported(Some(input), Some(output))
InferenceUsage::default() // only for deliberately unknown test providers
```

Do not use a bulk replacement that maps old miss counters to cache writes.
Update benchmark JSON keys to `total_input_tokens`, `cache_read_tokens`,
`cache_write_tokens`, and `cache_telemetry`.

- [ ] **Step 5: Run focused protocol and runtime targets**

```bash
bash scripts/cargo-agent.sh test -p fabric --test protocol_schema
bash scripts/cargo-agent.sh test -p fabric --test inference_contract
bash scripts/cargo-agent.sh test -p cognit --tests
bash scripts/cargo-agent.sh test -p executive --test turn_pipeline_order
bash scripts/cargo-agent.sh test -p executive --test turn_service_equivalence
bash scripts/cargo-agent.sh test -p interact --lib
```

Expected: PASS and the legacy-name grep returns only migration aliases/tests.

- [ ] **Step 6: Commit end-to-end usage propagation**

Commit with a full `refactor(inference)` message.

---

### Task 5: Correct Anthropic system and cache breakpoints

**Files:**
- Modify: `crates/cognit/src/adapters/inference/anthropic.rs`
- Create: `crates/cognit/tests/anthropic_cache_contract.rs`

- [ ] **Step 1: Build a local HTTP fixture test**

The fixture accepts one `/v1/messages` request, captures its JSON body, and
returns deterministic completion or SSE payloads. Send System, User, and tools
in deliberately noncanonical order. Assert:

```rust
assert_eq!(body["system"][0]["text"], "stable system");
assert!(body["messages"].as_array().unwrap().iter().all(|m| m["role"] != "system"));
assert_eq!(tool_names(&body), vec!["alpha", "zeta"]);
assert!(last_tool(&body)["cache_control"].is_object());
assert!(last_system_block(&body)["cache_control"].is_object());
assert!(last_user_block(&body).get("cache_control").is_none());
```

Use matching native usage fields for completion and SSE and assert equal
`InferenceUsage`.

- [ ] **Step 2: Run the fixture and observe failure**

```bash
bash scripts/cargo-agent.sh test -p cognit --test anthropic_cache_contract
```

Expected: FAIL because there is no system request field and streaming loses cache
usage.

- [ ] **Step 3: Implement request partitioning**

Add `system: Vec<ApiSystemBlock>` with skip-empty serialization. Partition every
System message content block into the system list; retain only User/Assistant in
messages. Apply the system breakpoint to the final supported System block and
the tool breakpoint to the final canonical tool. Never mark the current message.

- [ ] **Step 4: Implement one native usage mapper**

Use one `anthropic_usage(ApiUsage) -> anyhow::Result<InferenceUsage>` function in
both `complete` and `complete_stream`. Preserve cache fields from
`message_start`; merge the final output counter without erasing the initial
cache counters.

- [ ] **Step 5: Run the fixture three times**

```bash
for run in 1 2 3; do
  bash scripts/cargo-agent.sh test -p cognit --test anthropic_cache_contract || exit 1
done
```

Expected: all runs PASS with identical captured system/tool JSON digests.

- [ ] **Step 6: Commit Anthropic mapping**

Commit with a full `fix(anthropic)` message.

---

### Task 6: Terminal inference receipt schema and persistence port

**Files:**
- Create: `crates/fabric/src/types/inference_receipt.rs`
- Modify: `crates/fabric/src/types/mod.rs`
- Modify: `crates/fabric/src/lib.rs`
- Modify: `crates/fabric/src/types/model_projection.rs`
- Modify: `crates/fabric/src/include/turn.rs`
- Modify: `crates/fabric/src/types/session.rs`
- Modify: `crates/executive/src/composition/turn_service.rs`
- Modify: `crates/executive/src/adapters/session/event_sourced_store.rs`
- Modify exhaustive `ItemPayload` matches found by `rg -n 'ItemPayload::' crates/executive crates/interact`
- Test: `crates/fabric/tests/inference_contract.rs`
- Test: `crates/executive/src/composition/turn_service.rs`

- [ ] **Step 1: Add failing schema and persistence tests**

Construct a projection and terminal receipt with the same `inference_id`, append
both through RecordingTurnServices, and assert two distinct control items in
order. Assert schema validation rejects empty IDs/digests, an unknown schema
version, a failure without `failure_kind`, and success with `failure_kind`.

- [ ] **Step 2: Implement the typed terminal schema**

Create the exact design §7 fields, `InferenceTerminalStatus`, bounded validation,
and `SCHEMA_VERSION = 1`. Failure kinds are the normalized strings
`provider_transient`, `provider_terminal`, `context_overflow`, `timeout`,
`cancelled`, `invalid_request`, and `unknown`.

Extend `ModelContextProjectionReceipt` with:

```rust
pub system_prefix_digest: String,
pub tool_schema_digest: String,
```

Add `TurnServices::record_inference_receipt` with a compatibility no-op default.
Add `ItemPayload::InferenceReceipt { receipt }`; classify it as Control and
exclude it from model-history projection.

- [ ] **Step 3: Persist through RecordingTurnServices**

Materialize no model-visible content. Push the terminal receipt into canonical
items and delegate to the inner service exactly as capability and projection
receipts do.

- [ ] **Step 4: Run Fabric and Executive receipt tests**

```bash
bash scripts/cargo-agent.sh test -p fabric --test inference_contract
bash scripts/cargo-agent.sh test -p executive composition::turn_service --lib
bash scripts/cargo-agent.sh test -p executive --test session_use_case_port
```

Expected: PASS.

- [ ] **Step 5: Commit receipt schema and storage boundary**

Commit with a full `feat(inference)` message.

---

### Task 7: Exact-once inference recording wrapper

**Files:**
- Modify: `crates/cognit/src/harness/session.rs`
- Create: `crates/cognit/tests/inference_recording.rs`
- Modify: `crates/executive/tests/support/mock_llm_provider.rs`
- Modify: `crates/executive/tests/native_cognit_runtime.rs`

- [ ] **Step 1: Add recording fixtures**

Use a TurnServices fixture that records projection and terminal receipts, plus
providers for completion success/error and stream success/error/drop. Assert:

- each provider invocation has one projection and one terminal receipt;
- both receipts share `inference_id` and digests;
- the provider receives canonical tool order;
- failure classification is normalized;
- dropping a nonterminal stream records Cancelled with unknown usage;
- draining twice cannot duplicate a receipt.

- [ ] **Step 2: Refactor ProjectionRecordingLlm into a recorder**

Before each dispatch:

1. allocate inference ID;
2. canonicalize tools;
3. compute tool/system digests;
4. record projection with those values;
5. call the provider with canonical tools.

Keep an owned `Arc<Mutex<Vec<InferenceTerminalReceipt>>>` pending queue. Completion
pushes immediately. Stream wrapping owns a guard containing metadata, latest
usage, and an atomic terminal flag; Usage updates the guard, Done/error finalizes,
and Drop finalizes Cancelled when no terminal was observed.

- [ ] **Step 3: Drain before every run_turn exit**

After the ReAct future resolves, fails, or is cancelled, drain pending receipts
and await `services.record_inference_receipt` for each before returning the turn
result. Do not report a child/provider result before this authoritative terminal
snapshot has been persisted.

- [ ] **Step 4: Run Cognit and Native Cognit tests**

```bash
bash scripts/cargo-agent.sh test -p cognit --test inference_recording
bash scripts/cargo-agent.sh test -p cognit --tests
bash scripts/cargo-agent.sh test -p executive --test native_cognit_runtime
```

Expected: PASS; no invocation lacks or duplicates a terminal receipt.

- [ ] **Step 5: Commit terminal recording**

Commit with a full `feat(inference)` message.

---

### Task 8: Full validation and installed acceptance

**Files:**
- Modify: `docs/plans/2026-08-03-inference-cache-contract-implementation.md` only to mark executed steps and append actual evidence.

- [ ] **Step 1: Run the complete focused set sequentially**

```bash
bash scripts/cargo-agent.sh test -p fabric --test inference_contract
bash scripts/cargo-agent.sh test -p fabric --test protocol_schema
bash scripts/cargo-agent.sh test -p corpus tools::tools::registry::tests --lib
bash scripts/cargo-agent.sh test -p corpus --test capability_executor
bash scripts/cargo-agent.sh test -p cognit --tests
bash scripts/cargo-agent.sh test -p executive composition::turn_service --lib
bash scripts/cargo-agent.sh test -p executive --test native_cognit_runtime
bash scripts/cargo-agent.sh test -p executive --test turn_pipeline_order
bash scripts/cargo-agent.sh test -p executive --test turn_service_equivalence
bash scripts/cargo-agent.sh test -p interact --lib
bash scripts/aletheon.sh test architecture
bash scripts/cargo-agent.sh fmt --all -- --check
git diff --check
```

Expected: every command exits 0. Provider rejection, missing usage, duplicate
receipt, protocol mismatch, or architecture inventory drift is a failure.

- [ ] **Step 2: Inspect semantic legacy-name residue**

```bash
rg -n 'cache_hit_tokens|cache_miss_tokens' crates --glob '*.rs'
```

Expected: matches exist only in explicit backward-deserialization tests/aliases;
no production emission or aggregation uses the legacy names.

- [ ] **Step 3: Deploy through the mandatory installed path**

```bash
sudo bash scripts/aletheon.sh deploy
```

Expected: deployment, official Memory Agent smoke, and real official-socket
client request all PASS.

- [ ] **Step 4: Verify binary provenance and stability**

Record release, installed, machine daemon, and user daemon SHA-256; assert all are
identical. Record both PIDs and `NRestarts`, wait ten seconds, and assert both
services remain active with unchanged counters.

- [ ] **Step 5: Run three installed digest observations**

Issue three fresh official-client requests with unchanged model/profile/tool
configuration. Read their persisted projection and terminal inference receipts;
assert every run has matching projection/terminal IDs and that all three runs
have identical `system_prefix_digest` and `tool_schema_digest`. Cache telemetry
must be Reported, Unsupported, or Unknown rather than fabricated zero fields.

- [ ] **Step 6: Complete the plan evidence and commit**

Mark all plan steps complete, append exact commands/digests/PIDs/receipt IDs, run
`git diff --check`, inspect the staged diff, and commit with a full
`test(inference)` message.

- [ ] **Step 7: Merge and publish through the repository workflow**

Fetch `origin/dev`, merge it into the feature branch, push, open a PR to `dev`,
wait for required CI, repair any failure without bypassing checks, merge, delete
the feature branch, switch to `dev`, and verify a clean synchronized workspace.

---

## Execution evidence — 2026-08-03

### Implemented

- Fabric canonicalizes nested tool schemas, rejects duplicate/invalid names, and emits a domain-separated tool digest.
- Corpus model/profile snapshots are name-sorted; Cognit repeats canonicalization at every provider dispatch boundary.
- `InferenceUsage` is the only emitted usage schema across responses, streams, turn events, clients, metrics, and mocks.
- Anthropic System messages use the provider `system` field; cache breakpoints terminate at stable System and tool blocks.
- Model context projections and terminal inference receipts are distinct non-model-visible Session items linked by `inference_id`.
- The root daemon TurnServices path persists both items; this gap was found and fixed during installed acceptance.

### Deterministic validation

All commands exited 0:

```text
bash scripts/cargo-agent.sh test -p fabric --test inference_contract
bash scripts/cargo-agent.sh test -p fabric --test protocol_schema
bash scripts/cargo-agent.sh test -p corpus tools::tools::registry::tests --lib
bash scripts/cargo-agent.sh test -p corpus --test capability_executor
bash scripts/cargo-agent.sh test -p cognit --tests
bash scripts/cargo-agent.sh test -p executive composition::turn_service --lib
bash scripts/cargo-agent.sh test -p executive --test native_cognit_runtime
bash scripts/cargo-agent.sh test -p executive --test daemon_streaming_turn_e2e
bash scripts/cargo-agent.sh test -p executive --test turn_pipeline_order
bash scripts/cargo-agent.sh test -p executive --test turn_service_equivalence
bash scripts/cargo-agent.sh test -p interact --lib
bash scripts/aletheon.sh test architecture
bash scripts/cargo-agent.sh fmt --all -- --check
git diff --check
```

The reviewed architecture metric improved from `CORE_EXTERNAL_IDENTIFIER_HITS=26` to `25`; no architecture finding, dependency, or path was added.

### Installed-runtime evidence

`sudo bash scripts/aletheon.sh deploy` built and installed the release and restarted all services, but returned nonzero because the configured external GBrain OAuth client-credentials transport was unavailable. The daemon remained live with local authoritative memory ready and `supplemental_memory_spool=optional_degraded`. This external readiness failure is unresolved, so deployment acceptance is **not marked passed**.

After installation:

```text
release/install/machine/user SHA-256:
b1ea7f0be25e7c3653af8b1e827250cf2e701ed42895d0b4d185eab47cb92b23

machine daemon: PID 1876071, NRestarts 0, active/running
user daemon:    PID 1876090, NRestarts 0, active/running
```

Both PIDs and restart counters remained unchanged after ten seconds. Three fresh official-socket requests through `/usr/bin/aletheon` returned `cache-receipt-ok` without rendered/provider errors.

Persisted receipt pairs:

| Session | Inference ID | System digest | Tool digest | Status | Cache telemetry |
|---|---|---|---|---|---|
| `message-04300e1c-e979-4aaf-b78d-c91661b9599a` | `b95c5c83-3109-4408-a271-3495fe904d82` | `sha256:a91ba9a7a9b7542a1ac8dfd5a4fe6481d1e63b34b1f1df58274fce9fe0832a47` | `sha256:68d0b884af9ce799e107ecd7f8c11b4ead6969b3ffcaf0150de85440457bed8d` | succeeded | reported |
| `message-228ded12-87e5-4e49-bc23-cd0222a39c66` | `1f48a8e6-eca3-41fd-adfc-d0a9657e0714` | same | same | succeeded | reported |
| `message-fcea5ca1-b6e4-480b-890a-7dd17d211c2a` | `30d82920-3112-43ab-8ed1-72835399e582` | same | same | succeeded | reported |

Each Session stored `model_context_projection` immediately before its distinct `inference_receipt`, and each pair shared the same inference ID and digests.

### Remaining acceptance blocker

The required system deployment command must exit 0 before this plan can be called fully accepted or merged to `dev`. Current blocker: configured external GBrain MCP OAuth client-credentials failures make overall health `degraded`, and the deployment script requires `ready`.
