# P4: Brain Depth — Multi-Step Reasoning & Iterative Planning

**Date**: 2026-06-20
**Status**: ✅ Complete (PR #168 merged)
**Goal**: Transform the brain from single-step template to iterative, decomposing, self-correcting cognitive engine.

## Problem

The brain is template-based:
- ChainOfThought = 4-section format string, no real reasoning loop
- Planner = single-step plan wrapping, ignores LLM output
- Critic = one-shot structural checks, no iteration
- DualModel = two-pass with no feedback
- Learning rules injected as text but not consulted by planner

## 6 Changes

### 1. Plan-Critique-Revise Loop

In `BrainCore::think()`, after generating the initial plan, run a critique-revise loop:

```
plan = generate_plan(intent, reasoning)
for round in 0..max_rounds {
    critiques = critic.critique(plan)
    if no critical issues → break
    revised = llm.revise(plan, critiques)
    plan = parse_revised_plan(revised)
}
```

Max 3 rounds. Stops when no Critical-severity critiques remain.

### 2. Task Decomposition

New method `Planner::decompose_intent()`:
- Ask LLM to break complex intent into subtasks (JSON output)
- Parse JSON → Vec<(action, params)>
- If parse succeeds → `generate_multi_step_plan()`
- If parse fails → fallback to single-step `generate_plan()`

### 3. Planner Parses LLM Output

`Planner::parse_subtasks_from_reasoning()`:
- Extract JSON blocks from LLM reasoning text
- Validate structure (each subtask has action + params)
- Return `Option<Vec<(String, Value)>>`

### 4. DualModel Feedback

After executor produces plan, validate against planner's analysis:
- Check plan steps cover the analysis requirements
- If inconsistent → re-prompt executor with explicit correction
- Max 1 re-prompt to avoid loops

### 5. Learning Integration

At the start of `think()`:
- Query `Learner::rules_for_context(intent.description)`
- Format rules as bullet points
- Inject into LLM prompt as "Learned Rules" section

### 6. Iterative Reasoning Loop

Implemented by #1 (Plan-Critique-Revise is the iterative loop).

## Files to Modify

| File | Changes |
|---|---|
| `crates/aletheon-brain/src/core/mod.rs` | think() rewrite: add refinement loop, learning integration, decomposition |
| `crates/aletheon-brain/src/core/planner.rs` | decompose_intent(), parse_subtasks_from_reasoning() |
| `crates/aletheon-brain/src/core/critic.rs` | Add severity levels to Critique struct |
| `crates/aletheon-brain/src/bridge/dual_model.rs` | Add validation + re-prompt logic |
| Tests | Unit + integration tests |

## Implementation Order

1. Add severity to Critique + revision prompt builder
2. Plan-Critique-Revise loop in think()
3. Task decomposition (LLM output parsing + multi-step plan)
4. Learning integration
5. DualModel feedback
6. Tests
7. Validate: cargo test --workspace
