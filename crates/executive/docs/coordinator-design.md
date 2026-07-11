# Coordinator Design

## Definition

The Coordinator is a **temporary arbitrator**, not a supreme authority. It
sits between CognitCore (which produces plans) and SelfField (which reviews
intents) and synthesizes their outputs into a single actionable decision for
the Engine.

The Coordinator owns no persistent state. It reads inputs, applies a decision
tree, and returns an `ArbitrationResult`. It can be replaced, bypassed, or
reconfigured without data loss — all state it uses comes from external sources
(MemoryContext, Verdict, Plan).

## Relationship to Engine

```
User Intent
    |
    v
CognitCore.think() -> Plan
    |
    v
SelfField.review() -> Verdict
    |
    v
Coordinator::arbitrate(Verdict, Plan, MemoryContext) -> ArbitrationResult
    |
    v
Engine acts on result
    - Execute         -> Body.execute()
    - Deny            -> report to user
    - Delay           -> re-queue
    - SandboxFirst    -> sandbox.run() -> if pass -> Body.execute()
    - AskConfirmation -> prompt user
    - Reflect         -> CognitCore.think() again
    - Mutate          -> rewrite plan, re-arbitrate
```

The Engine does **not** delegate control flow permanently to the Coordinator.
Each arbitration is a single call. The Engine retains scheduling authority.

## Input Specification

### Verdict (from SelfField)

| Variant | Meaning |
|---------|---------|
| `Allow` | SelfField sees no policy violation |
| `AllowWithModification` | Allowed if plan is adjusted per the modification |
| `Deny` | Hard refusal — no override |
| `RequireConfirmation` | User must explicitly approve |
| `SandboxFirst` | Must prove safe in sandbox before real execution |
| `Delay` | Wait until a condition is met |

### Plan (from CognitCore)

- `risk_level: RiskLevel` — aggregated risk assessment (None/Low/Medium/High/Critical)
- `reasoning: String` — why this plan was chosen
- `steps`, `cost_estimate`, `alternatives` — available but not used in current arbitration logic

### MemoryContext (from execution history)

| Field | Type | Purpose |
|-------|------|---------|
| `similar_action_failures` | `usize` | Count of past failures for this action type |
| `user_has_overridden` | `bool` | User previously overrode a denial for this action type |
| `prior_sandbox_success` | `bool` | This action type passed sandbox before |
| `last_failure_at` | `Option<i64>` | Timestamp of most recent failure |

## Output Specification

`ArbitrationResult` is one of:

| Variant | Engine Action |
|---------|---------------|
| `Execute` | Proceed with plan as-is |
| `Deny { reason }` | Refuse, report reason to user |
| `Delay { reason, until }` | Re-queue with condition |
| `SandboxFirst { reason }` | Run in sandbox, promote on pass |
| `AskConfirmation { reason }` | Prompt user for explicit approval |
| `Reflect { reason }` | Feed back to CognitCore for another think cycle |
| `Mutate { reason, suggested_modification }` | Rewrite plan, then re-arbitrate |

## Decision Tree

Evaluated in order — first match wins:

```
1. Verdict::Deny?
   YES -> Deny (hard stop, no override)

2. Verdict::SandboxFirst?
   YES -> SandboxFirst

3. Verdict::Delay?
   YES -> Delay

4. Verdict::RequireConfirmation?
   YES -> AskConfirmation

5. Verdict::AllowWithModification?
   YES -> Mutate (pass modification suggestion)

6. Verdict::Allow + Plan.risk == Critical?
   YES -> SandboxFirst (Critical always sandboxed)

7. Verdict::Allow + Plan.risk == High + past failures?
   YES -> AskConfirmation
         UNLESS user_has_overridden -> Execute (trust the user)

8. Verdict::Allow + Plan.risk in [Medium, Low, None]?
   -> Execute
```

## Design Rationale

**Why not a god object?** The Coordinator is a pure function with context
(`arbitrate`). It does not hold state, schedule work, or own resources.
This makes it trivially testable, replaceable, and composable.

**Why not just use Verdict directly?** SelfField's Verdict captures policy
concerns but not execution history. The Coordinator enriches the decision
with memory context (past failures, user overrides) that SelfField does
not have access to.

**Why SandboxFirst for Critical even with Allow?** Risk level is an
orthogonal signal from policy compliance. Something can be policy-legal
but still dangerous. Critical risk always warrants sandboxing as a
defense-in-depth measure.

**Why trust user overrides?** If a user has previously overridden a denial
for an action type, they have demonstrated informed consent. Forcing
confirmation on every high-risk retry would be paternalistic and
anti-productive. The override flag is the user's opt-out of the safety net.
