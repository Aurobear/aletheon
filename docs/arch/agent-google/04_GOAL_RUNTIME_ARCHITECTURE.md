# Goal Runtime Architecture

> **Status:** Canonical goal semantics; implementation is partially complete
>
> **Rewritten:** 2026-07-19

## Goal Definition

A Goal is a durable intent that may require multiple supervised attempts across
time. It is not a Prompt, chat message, Runtime session or background task
handle.

Current typed Goal creation/list/action ports are defined at
`crates/executive/src/service/goal_service.rs:43-72`. The SQLite objective store,
migrations, attempts, retry, verification and worker modules are assembled at
`crates/executive/src/impl/goal/mod.rs:1-37`.

## Authority

Executive owns Goal state and transitions. Kernel owns the lifecycle primitives
for each attempt. Runtime may execute an attempt but cannot own the Goal or mark
it globally complete.

```text
Goal authority (Executive)
    -> compile next bounded attempt
    -> Kernel Operation + admission
    -> Cognit or selected Runtime
    -> evidence and untrusted receipt
    -> independent verification
    -> atomic Goal transition
```

## State Model

A Goal must distinguish at least:

```text
proposed
active
waiting
paused
completed
failed
cancelled
```

`waiting` includes a typed reason such as approval, external event, retry time,
resource availability or user input. State transitions require expected-version
checks; current service errors already distinguish version conflict at
`crates/executive/src/service/goal_service.rs:31-40`.

## Attempts

Every attempt records:

- Goal ID and expected Goal version;
- Operation and Process identity;
- bounded input and acceptance criteria;
- capability scope and budget;
- selected Cognit/Runtime executor;
- events, tool evidence and artifact references;
- terminal reason and usage;
- independent verification report.

Retry creates a new attempt. It does not overwrite the failed attempt or reuse a
stale permit.

## Verification

Completion requires acceptance evidence evaluated outside the executor that
claims success. For coding work this includes deterministic commands and the
workspace diff. For provider side effects it includes provider receipt and
idempotency identity. For user-facing goals it may require explicit user
acceptance.

A Runtime `SucceededUnverified` receipt is therefore expected evidence, not a
Goal transition by itself.

## Recovery

After restart, reconciliation must decide from durable facts whether an attempt:

- never started and may be admitted;
- is resumable through the same external Runtime identity;
- lost its executor and must fail/retry;
- already settled and must not execute again;
- waits for an unexpired approval or external event.

No recovery path may allocate two active attempts for the same exclusive work.

## Channels

Channels may create, inspect, pause, resume or cancel Goals only through typed
Executive use cases. Gmail drafts and Telegram commands are adapters; neither is
the Goal authority.

## Non-Goals

- one OS process per Goal;
- one crate called `goal-runtime`;
- treating every chat turn as a durable Goal;
- allowing a model or Runtime to self-approve;
- hiding blocked/waiting states behind repeated retries;
- keeping only the latest attempt result.

## Production Acceptance

- versioned transitions reject stale writers;
- cancellation reaches live Operation/Process and settles once;
- restart recovery does not duplicate attempts;
- budgets and leases settle on every terminal path;
- success requires independent evidence;
- channel commands enforce principal ownership;
- complete history is replayable from the durable authority.
