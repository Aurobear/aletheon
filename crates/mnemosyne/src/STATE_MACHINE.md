# Mnemosyne service state machine

`lifecycle.rs` is the operation-order owner. Each public memory operation creates a
short-lived lifecycle and mutates it only through `MemoryOperationLifecycle::apply`.

```text
write:  Ready -> LocalWrite -> Projection -> SupplementalWrite -> Completed|Degraded
recall: Ready -> LocalRecall -> SupplementalRecall -> Merging -> Completed|Degraded
worker: Ready -> Reconciliation -> Completed
forget: Ready -> Retention -> Completed
```

Local repository effects always run before optional supplemental effects. Missing,
timed-out, or failed supplemental memory therefore never removes a successful local
write or recall. Local failure is terminal. Supplemental recall degradation still
passes through merge so local results remain available and carry a degraded-source
marker.

Repository, projection, supplemental transport, reconciliation, and retention are
effect ports; the reducer performs no I/O. Record IDs, retention request IDs, spool
operation IDs, and tombstone outbox rows are persistence/idempotency keys. Repeated
terminal events fail closed. Budget expiry cancels supplemental recall and degrades
to local results; reconciliation shutdown uses its supervisor cancellation token.
After a crash, durable local rows and spool/outbox rows are replayed by reconciliation
rather than by resuming an in-memory lifecycle.
