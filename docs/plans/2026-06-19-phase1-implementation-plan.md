# Phase 1 Implementation Plan: LLM Working + Basic Interaction

**Date:** 2026-06-19
**Design spec:** [2026-06-19-cli-agent-design.md](./2026-06-19-cli-agent-design.md)
**Goal:** `aletheon run "hello"` calls LLM and returns result (standalone, no daemon required)
**Estimated effort:** ~4.5 hours

---

## Key Finding

Most infrastructure already exists:
- `LlmProvider` trait: `aletheon-brain/src/impl/llm/provider.rs`
- `ProviderRegistry`: `aletheon-brain/src/impl/provider_registry.rs`
- OpenAI-compatible provider with streaming: `aletheon-brain/src/impl/llm/openai_provider.rs`
- Config layer: `aletheon-brain/src/config/mod.rs` + `aletheon-runtime/src/core/config.rs`
- CLI with `single_message()`: `aletheon-body/src/impl/cli/mod.rs`

**Gap:** `aletheon run "hello"` currently requires daemon. Phase 1 makes it standalone.

---

## Tasks

### Task 1: Add `run` subcommand to CLI
**File:** `crates/aletheon-body/src/impl/cli/mod.rs`
**Effort:** 30 min | **Priority:** P0

- Add `Run` variant to `Command` enum with `message` positional arg
- Route to new `run_single_shot()` function:
  1. Load `AppConfig` via `load_layered(None)`
  2. Build `ProviderRegistry` from config
  3. Resolve provider+model
  4. Build `Message::user(text)` + system prompt
  5. Call `llm.complete()` or `llm.complete_stream()`
  6. Print response to stdout

### Task 2: Add `--model` flag
**File:** `crates/aletheon-body/src/impl/cli/mod.rs`
**Effort:** 15 min | **Priority:** P1

- Add `model: Option<String>` to `Args` with `#[arg(short = 'M', long)]`

### Task 3: Ensure `.env` loading in standalone mode
**File:** `crates/aletheon-body/src/impl/cli/mod.rs`
**Effort:** 15 min | **Priority:** P0

- Load `~/.aletheon/.env` in `run_single_shot()`
- Extract from `aletheon-runtime/src/impl/daemon/mod.rs:load_dotenv()` or duplicate

### Task 4: Update `setup.sh` wrapper
**File:** `setup.sh`
**Effort:** 15 min | **Priority:** P1

- `aletheon run` (no args) → daemon + TUI (existing)
- `aletheon run "msg"` → single-shot (new)

### Task 5: Enhance `aletheon-abi` LLM types
**File:** `crates/aletheon-abi/src/llm_types.rs`
**Effort:** 30 min | **Priority:** P2

- Add `ChatRequest`, `ChatResponse`, `LlmEvent`, `ModelInfo` types
- New additions only, no breaking changes

### Task 6: Add `ModelInfo` to `LlmProvider` trait
**File:** `crates/aletheon-brain/src/impl/llm/provider.rs`
**Effort:** 15 min | **Priority:** P2

- Add `fn model_info(&self) -> ModelInfo` with default impl

### Task 7: Dependency check (BLOCKER)
**File:** `crates/aletheon-body/Cargo.toml`
**Effort:** 15 min | **Priority:** P0

- Verify `aletheon-body` → `aletheon-brain` dependency path
- If circular: move `run` logic to `aletheon-cli` binary crate (recommended)

### Task 8: System prompt for standalone mode
**File:** `crates/aletheon-body/src/impl/cli/mod.rs`
**Effort:** 15 min | **Priority:** P1

- Prepend `"You are Aletheon, a helpful AI assistant."` system message

### Task 9: Streaming output
**File:** `crates/aletheon-body/src/impl/cli/mod.rs`
**Effort:** 30 min | **Priority:** P1

- Use `llm.complete_stream()` for real-time token output
- Consume `StreamChunk::TextDelta` → print to stdout

### Task 10: Integration tests
**File:** `crates/aletheon-body/src/impl/cli/mod.rs`
**Effort:** 30 min | **Priority:** P1

- Args parsing tests
- Config loading + provider resolution mock tests

### Task 11: Update `default.toml`
**File:** `config/default.toml`
**Effort:** 10 min | **Priority:** P1

- Add `[[providers]]` and `[model_aliases]` sections

### Task 12: End-to-end verification
**Effort:** 30 min | **Priority:** P0

```bash
export MIMO_API_KEY="tp-..."
cargo build
./target/debug/aletheon run "what is 1+1"       # expect: "2"
./target/debug/aletheon run --model flash "hello" # expect: greeting
aletheon run                                      # expect: interactive TUI
```

---

## Dependency Graph

```
Task 5 (abi types) ──────────┐
                              ├─→ Task 6 (model_info)
Task 7 (dep check) ──────────┤
                              ├─→ Task 1 (run subcommand)
Task 2 (--model flag) ───────┤
Task 3 (.env loading) ───────┤
Task 8 (system prompt) ──────┤
                              ├─→ Task 9 (streaming)
Task 4 (setup.sh) ───────────┤
Task 11 (default.toml) ──────┤
                              └─→ Task 10 (tests) → Task 12 (e2e)
```

## Critical Path

**Task 7** is the blocker. Recommended: put `run_single_shot` in `crates/binaries/aletheon-cli/src/main.rs` to avoid body→brain dependency issues.

## Recommended Execution Order

1. Task 7 (dep check) — resolve blocker
2. Task 5 (abi types) — foundation types
3. Task 3 (.env loading) — env setup
4. Task 1 (run subcommand) — core feature
5. Task 2 (--model flag) — UX polish
6. Task 8 (system prompt) — quality
7. Task 9 (streaming) — UX polish
8. Task 11 (default.toml) — config
9. Task 4 (setup.sh) — wrapper
10. Task 10 (tests) — validation
11. Task 12 (e2e) — final verification
