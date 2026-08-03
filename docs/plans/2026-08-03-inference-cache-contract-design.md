# Inference Cache Contract Design

**Date:** 2026-08-03

**Status:** Selected design. The user approved the recommended P0 scope after the
cache audit against `dev@8f69268c61eab147caa7cde15ace370d3380fe00`.

**Scope:** Make provider-prefix caching deterministic, observable, and
semantically comparable across streaming and non-streaming inference. This design
does not add an answer cache, a recall cache, a tool-result cache, a machine-wide
provider runtime, or a new top-level crate.

## 1. Requirement and code anchors

The approved P0 work has four independently testable requirements:

1. **P0-A — deterministic model capabilities.** Every model-visible
   `ToolDefinition` list must have one canonical order and one canonical JSON
   representation. Duplicate tool names fail closed. Each inference records a
   `tool_schema_digest` and an actual-system-prefix digest.
2. **P0-B — one usage meaning.** Streaming and non-streaming paths use one
   `InferenceUsage` contract. Cache reads, cache writes, uncached input, total
   logical input, and output remain dimensionally separate. Missing provider
   telemetry is `unknown`, never fabricated as zero.
3. **P0-C — correct Anthropic mapping.** System messages use Anthropic's system
   parameter; stable tool and system regions receive explicit cache breakpoints;
   dynamic conversation messages remain after them. Both completion modes map
   Anthropic cache read/write/input fields identically.
4. **P0-D — terminal inference evidence.** Every attempted provider inference
   produces a typed terminal receipt linked to the pre-call context projection by
   one `inference_id`. The receipt records effective provider/model, canonical
   digests, terminal status, and authoritative observed usage.

The current implementation establishes the change points:

- `crates/corpus/src/tools/tools/registry.rs:39-68` derives both public tool lists
  from `HashMap::values()` without ordering.
- `crates/executive/src/host/daemon/bootstrap/request.rs:1007-1014` forwards that
  list directly into Goal worker runtimes, although native profile paths and the
  extension catalog already have partial deterministic ordering.
- `crates/cognit/src/adapters/inference/anthropic.rs:200-258` folds System into
  User and attaches cache control to the last tool and the last dynamic message.
- `crates/cognit/src/adapters/inference/anthropic.rs:343-349` maps cache creation
  to `cache_miss_tokens`.
- `crates/fabric/src/types/llm_types.rs:22-40` exposes streaming usage without
  cache dimensions, while `LlmResponse` separately owns hit/miss fields at
  `crates/fabric/src/types/llm_types.rs:129-139`.
- `crates/cognit/src/harness/linear/tool_exec.rs:142-153` fabricates zero cache
  counters for streaming events.
- `crates/fabric/src/types/attempt.rs:57-80` already models cache read/write as
  optional runtime observations, proving that unknown values are an established
  contract.
- `crates/cognit/src/harness/session.rs:24-105` owns the current pre-call model
  projection boundary and creates an `inference_id`, but does not carry that ID
  through provider completion.
- `crates/fabric/src/types/model_projection.rs:7-17` already defines the
  reconstructable context projection receipt that the terminal receipt must
  reference rather than replace.

## 2. Considered approaches

### Approach A — patch each provider and UI independently

Sort tools in Corpus, add Anthropic fields, and extend TUI counters in place.
This is fast but preserves multiple usage vocabularies and lets new provider
paths bypass canonicalization. It does not meet P0-B or P0-D.

### Approach B — canonical Fabric contract with Cognit terminal recording

Put provider-neutral `InferenceUsage`, cache telemetry status, canonical tool
serialization, digests, and inference receipts in Fabric. Cognit canonicalizes
immediately before provider dispatch, adapters map their native APIs, and
Executive persists the resulting receipts. Existing ContextAssembler and
ProjectionReceipt remain the selection and pre-call evidence boundaries.

This is the selected approach. It follows current crate ownership and avoids a
new cache authority.

### Approach C — introduce a cache/inference runtime crate

A new runtime could centralize provider scheduling and cache metrics, but it
would combine this P0 correctness repair with the later machine-wide scheduling
work. It adds an unnecessary top-level authority and is rejected for this scope.

## 3. Architecture and ownership

```text
Executive context selection and trusted projections
        │
        ▼
Cognit inference dispatch
  ├─ canonicalize system fragments
  ├─ canonicalize ToolDefinitions
  ├─ compute system_prefix_digest + tool_schema_digest
  └─ create shared inference_id
        │
        ├──────── pre-call ────────► ModelContextProjectionReceipt
        │                              (existing, reconstructable)
        ▼
Provider adapter
  OpenAI / Anthropic / Ollama
        │
        ├─ non-stream response ─┐
        └─ terminal stream ─────┤
                               ▼
                    InferenceTerminalReceipt
                    ├─ provider/model facts
                    ├─ terminal status
                    ├─ canonical digests
                    └─ InferenceUsage
                               │
                               ▼
                    Executive canonical item/event persistence
```

Ownership is explicit:

- **Fabric** owns wire-safe inference types and deterministic canonicalization
  rules because `ToolDefinition`, `LlmResponse`, and `StreamChunk` already live
  there.
- **Cognit** owns context compilation at the provider boundary and provider API
  mappings.
- **Executive** owns durable Session/Event/Attempt persistence and aggregation;
  it does not infer missing provider cache values.
- **Provider caches** remain non-authoritative external optimizations. Receipts
  are authoritative only about what the host sent and what the provider
  reported.

## 4. Canonical tool and prefix contract

Fabric exposes one canonicalizer:

```rust
pub fn canonicalize_tool_definitions(
    definitions: &[ToolDefinition],
) -> Result<Vec<ToolDefinition>, ToolDefinitionCanonicalizationError>;

pub fn tool_schema_digest(definitions: &[ToolDefinition])
    -> Result<String, ToolDefinitionCanonicalizationError>;
```

Canonicalization rules are:

1. Clone definitions and recursively sort every JSON object key.
2. Preserve JSON array order because arrays can be semantically ordered.
3. Sort definitions by exact UTF-8 tool name.
4. Reject empty/control-character names and duplicate names.
5. Serialize the canonical vector with `serde_json::to_vec`.
6. Hash domain-separated bytes with SHA-256 and return `sha256:<hex>`.

`ToolRegistry::definitions()` and `profile_definitions()` return name-sorted
snapshots for deterministic discovery and bootstrap. Cognit repeats the full
canonicalization immediately before dispatch so MCP tools, AgentControl merges,
profiles, tests, and future sources cannot bypass the final boundary.

The system digest covers the exact ordered System content blocks submitted to
that inference, after provider-neutral compilation but before provider-specific
mapping. It is named `system_prefix_digest`; it does not claim that workspace- or
profile-specific content is globally reusable.

The existing Mnemosyne `capability_digest` remains untouched because it hashes a
supplemental memory grant and has a different semantic contract.

## 5. Unified usage semantics

Fabric replaces the split `Usage` plus `cache_hit_tokens/cache_miss_tokens`
contract with:

```rust
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InferenceUsage {
    pub total_input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub uncached_input_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub cache_telemetry: CacheTelemetry,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheTelemetry {
    Reported,
    Unsupported,
    #[default]
    Unknown,
}
```

Rules:

- `None` means unavailable or not authoritatively observed.
- `Some(0)` means the provider explicitly reported zero.
- `Reported` requires all cache dimensions that the provider contract promises;
  provider-specific unsupported dimensions remain `None`.
- No code derives cache write from cache miss or derives provider support from a
  zero counter.
- Aggregation sums only observed values and separately records whether every
  contributing inference was observed. It never converts a partial sum into a
  complete total.

Provider mappings:

- **Anthropic:** `uncached_input_tokens = input_tokens`, cache read/write come
  from their native fields, and total input is the checked sum of uncached,
  read, and write.
- **OpenAI-compatible:** total input is `prompt_tokens`; cache read is
  `cached_tokens` when present; uncached input is total minus read; cache write
  remains unknown because that API does not report it.
- **Ollama/local:** input/output are recorded when supplied; cache telemetry is
  `Unsupported` unless the provider adds an authoritative field.

Both `LlmResponse` and `StreamChunk::Usage` carry `InferenceUsage`. Cognit's
internal usage event, Fabric turn stream event, client event, Attempt usage, and
evaluation metrics consume the same field names. The old hit/miss names are
removed rather than maintained as a second semantic system.

## 6. Anthropic request compilation

The Anthropic request type gains a dedicated optional `system` block list.
Provider-neutral messages are partitioned before serialization:

```text
System messages -> ApiRequest.system
User/Assistant messages -> ApiRequest.messages
```

Cache controls are placed at:

1. the final canonical tool definition, when tools are nonempty;
2. the final System content block, when System content is nonempty.

No automatic cache control is added to the current user message. Dynamic Agora,
Memory, Dasein, history, tool results, and current input remain in `messages`.
A future context compiler may introduce an explicit conversation breakpoint, but
this P0 change does not guess which dynamic history is reusable.

Streaming parses the full usage object from `message_start` and terminal usage
updates. The terminal emitted `InferenceUsage` must equal the non-stream mapping
for the same provider payload.

## 7. Terminal inference receipt

Fabric adds a distinct terminal schema rather than overloading a public Turn or
Session progress event:

```rust
pub struct InferenceTerminalReceipt {
    pub schema_version: u32,
    pub inference_id: String,
    pub operation_id: String,
    pub provider_id: String,
    pub model_id: String,
    pub system_prefix_digest: String,
    pub tool_schema_digest: String,
    pub status: InferenceTerminalStatus,
    pub usage: InferenceUsage,
    pub failure_kind: Option<String>,
}
```

Terminal statuses are `Succeeded`, `Failed`, `Cancelled`, and `TimedOut`.
Provider error prose is not stored; `failure_kind` is a bounded typed/normalized
host classification.

`ProjectionRecordingLlm` becomes an inference-recording wrapper:

1. Canonicalize tools and compute both digests.
2. Allocate one `inference_id`.
3. Record `ModelContextProjectionReceipt` with that ID before dispatch.
4. For `complete`, record exactly one terminal receipt after response/error.
5. For `complete_stream`, wrap the returned stream and record exactly one
   terminal receipt after authoritative terminal usage plus Done, or after
   stream error/drop/cancellation.

A successful stream without an authoritative usage frame succeeds with unknown
usage rather than fabricated zeros. Receipt recording failures remain visible
host failures; an asynchronous stream return is not treated as terminal
receipt success.

Executive persists the receipt as its own canonical item payload linked by
`inference_id`. Projection and terminal inference receipts are different event
semantics and therefore remain different schemas.

## 8. Compatibility and migration

This is an internal protocol migration completed atomically in one branch:

- Update all constructors and mock providers to use `InferenceUsage`.
- Update protocol schema fixtures and serialization tests.
- Update TUI and benchmark output to the new read/write/unknown vocabulary.
- Historical persisted events with hit/miss fields remain readable through a
  narrowly scoped deserialization migration that maps old hit to read and maps
  old miss to no authoritative cache-write value. New serialization never emits
  the legacy fields.
- Historical `LlmResponse` fixtures may map known input/output values, but cache
  telemetry remains `Unknown` unless the old source can be interpreted without
  guessing.

## 9. Error handling

- Duplicate or invalid ToolDefinitions fail before any provider request.
- Digest/canonicalization failure produces a failed inference receipt and no
  provider call.
- Arithmetic overflow while deriving total input fails closed as invalid
  provider telemetry.
- Missing provider cache fields remain unknown and do not fail inference.
- Contradictory provider fields, such as cached input greater than total input,
  produce a failed inference result rather than saturating silently.
- A terminal receipt is emitted at most once per `inference_id`.

## 10. Validation and acceptance

Deterministic tests must prove:

1. differently inserted ToolDefinitions yield identical canonical bytes and
   digest;
2. nested JSON object key order does not affect the digest;
3. array order does affect the digest;
4. duplicate tools fail before provider dispatch;
5. Registry, Profile, AgentControl merge, Goal worker, and MCP-derived catalogs
   reach the same canonical order;
6. Anthropic uses the system field and never folds System into User;
7. Anthropic cache controls end at tools and System, not the current user input;
8. equivalent streaming and non-streaming payloads produce equal
   `InferenceUsage`;
9. OpenAI, Anthropic, and unsupported-provider unknown semantics are distinct;
10. every complete/stream success and failure produces exactly one terminal
    receipt linked to its projection receipt;
11. historical usage events deserialize without claiming cache writes;
12. formatting, protocol schema, architecture checks, and focused crate tests
    pass.

Installed acceptance runs the production deployment gate because this change
modifies provider adapters, shared protocol types, daemon events, persistence,
and client behavior:

```bash
sudo bash scripts/aletheon.sh deploy
```

After deployment:

- release, `/usr/bin/aletheon`, and both running daemon executables have the same
  SHA-256;
- machine and user restart counters remain stable;
- a real request through `/usr/bin/aletheon` and the official user socket
  completes without rendered inference errors;
- three consecutive installed runs with unchanged configuration report identical
  `system_prefix_digest` and `tool_schema_digest`;
- streaming usage reports cache telemetry as observed or unknown, never as an
  invented zero.

## 11. Explicit non-goals

This design does not implement:

- final-answer or semantic response caching;
- Mnemosyne recall result caching or scope generations;
- Corpus tool-result caching or `CachePolicy`;
- model KV storage inside Aletheon;
- machine-wide provider concurrency/cooldown authority;
- a second ContextAssembler or Session/Goal/Agent authority;
- a new top-level crate.

Those remain later workstreams after P0 metrics are trustworthy.
