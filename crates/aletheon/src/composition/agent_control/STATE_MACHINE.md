# Agent Control state machine

`lifecycle.rs` is the single lifecycle-policy owner. Repository `transition` is
the only durable mutation entry point and interprets its typed effects.

| State | Accepted events | Next state |
|---|---|---|
| `Queued` | `Start`, `Cancel`, `Fail`, `Interrupt` | `Running` or terminal |
| `Running` | `Wait`, `Succeed`, `Fail`, `Cancel`, `Interrupt` | `Waiting` or terminal |
| `Waiting` | `Resume`, `Succeed`, `Fail`, `Cancel`, `Interrupt` | `Running` or terminal |
| terminal | none | none |

Terminal states are `Succeeded`, `Failed`, `Cancelled`, and `Interrupted`.
Repeated and post-terminal events fail closed rather than silently succeeding.

Admission guards identity depth, parents, budgets, leases, and storage before a
`Queued` record exists. Success additionally requires a validated result. Accepted
events emit `PersistStatus`; initial start emits `MarkStarted`; terminal events emit
`MarkTerminal`. SQLite interprets them atomically using `(agent_id, expected_status)`
and row version as concurrency/idempotency keys. Settlement has durable, exactly-once
receipts of its own.

Cancellation is explicit from every non-terminal state. A wait timeout is an
observation, not a lifecycle mutation. Startup recovery reads open records and uses
the same repository entry to interrupt or adopt an operation terminal state. A crash
before commit leaves the old state; after commit, replay fails compare-and-swap.

Messaging has its own durable delivery state. Runtime, messaging, and settlement
ports never directly mutate the run lifecycle.
