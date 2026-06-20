# P2: SelfField → Behavior Integration

**Date**: 2026-06-20
**Status**: 🟡 In Progress
**Goal**: Make all 6 SelfField verdict types actually influence behavior in the ReAct loop.

## Problem

SelfField produces 6 verdict types (`Allow`, `AllowWithModification`, `Deny`, `RequireConfirmation`, `SandboxFirst`, `Delay`), but `process_react()` only handles `Deny`. The other 5 are dead letters — silently treated as "proceed normally."

`BehaviorPathRouter` correctly maps all verdicts to behavior paths, but it's wired to the old `process()` method, not the live `process_react()` path.

## Design

### VerdictHandler Trait

```rust
/// Handles a SelfField verdict before execution.
pub trait VerdictHandler {
    fn handle(&self, verdict: &Verdict, intent: &Intent, ctx: &Context) -> VerdictAction;
}

/// What to do after handling a verdict.
pub enum VerdictAction {
    /// Proceed with execution (possibly with modified intent)
    Proceed { modified_intent: Option<Intent> },
    /// Short-circuit with a response
    ShortCircuit { response: String },
    /// Run in sandbox first, then proceed if test passes
    SandboxThenProceed { sandbox_config: SandboxConfig },
}
```

### Verdict → Action Mapping

| Verdict | VerdictAction | Behavior |
|---|---|---|
| Allow | Proceed { None } | Normal execution |
| AllowWithModification | Proceed { Some(mod) } | Use SelfField's rewritten intent |
| Deny | ShortCircuit { reason } | Return denial message |
| RequireConfirmation | Callback → Proceed or ShortCircuit | Ask user, then decide |
| SandboxFirst | SandboxThenProceed → Proceed | Test in sandbox first |
| Delay | ShortCircuit { "delayed..." } | Return delay message |

### RequireConfirmation Callback

```rust
/// Callback for user confirmation. Returns true to proceed, false to deny.
pub type ConfirmCallback = Box<dyn Fn(&str, &str) -> bool + Send + Sync>;
```

The ReActLoop holds an optional `ConfirmCallback`. When `RequireConfirmation` fires:
1. Call callback with (reason, risk_level)
2. If callback returns true → Proceed
3. If callback returns false → ShortCircuit { "User denied: ..." }
4. If no callback → ShortCircuit { "Confirmation required but no handler" }

### SandboxFirst Implementation

Reuses MorphogenesisPipeline's sandbox infrastructure:
1. Create a sandbox from the current genome
2. Run the tool call in sandbox mode
3. If sandbox passes → Proceed with real execution
4. If sandbox fails → ShortCircuit { "Sandbox test failed: ..." }

### Files to Modify

| File | Change |
|---|---|
| `aletheon-abi/src/self_field.rs` | Add VerdictHandler trait + VerdictAction enum |
| `aletheon-runtime/src/core/verdict_handler.rs` | NEW: DefaultVerdictHandler implementation |
| `aletheon-runtime/src/core/orchestrator.rs` | Wire VerdictHandler into process_react() |
| `aletheon-runtime/src/core/react_loop.rs` | Add confirm_callback field |
| `aletheon-runtime/src/core/mod.rs` | Export new module |
| Tests | Unit tests per verdict + integration tests |

### Implementation Order

1. Add VerdictHandler trait + VerdictAction to aletheon-abi
2. Implement DefaultVerdictHandler in aletheon-runtime
3. Wire into orchestrator.process_react()
4. Add ConfirmCallback to ReActLoop
5. Add SandboxFirst logic (reuse Morphogenesis)
6. Tests for all 6 verdicts
7. Validate: cargo test --workspace

## Philosophy Connection

- **Spinoza's conatus**: Self-preservation through boundary enforcement
- **Heidegger's Sorge**: Care as the structure that filters action
- **Growth principle**: The agent learns its boundaries through verdict handling
