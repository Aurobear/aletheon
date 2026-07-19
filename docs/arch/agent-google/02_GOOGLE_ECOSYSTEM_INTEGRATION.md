# Google Ecosystem Integration

> **Status:** Mixed current implementation and bounded target
>
> **Rewritten:** 2026-07-19

## Ownership

Google is an external capability provider, not a new domain authority:

```text
Gateway input / Executive admin
          |
          v
Executive account and scope checks
          |
          v
Corpus Google tool adapter
          |
          v
Google API
```

Current adapters for Gmail, Calendar, Drive, OAuth and sync exist under
`crates/corpus/src/tools/google/`. Executive performs account binding and
scope-gated tool registration in
`crates/executive/src/service/request_use_cases.rs:808-855`.

## Identity and Authorization

- A Google account is bound to an Aletheon principal.
- OAuth scopes are explicit and least-privilege.
- Read scope does not imply write, send or delete authority.
- Refresh tokens are credentials, never Prompt or Agora content.
- Revocation disables tools and future sync; cached durable records retain
  provenance and retention policy.

## Capability Classes

```text
Read
  Gmail search/read
  Calendar list
  Drive metadata/content read

Draft or propose
  Gmail draft
  Calendar change proposal
  Drive change proposal

Commit side effect
  send mail
  create/update event
  modify/delete Drive content
```

Read operations still pass capability admission. Draft/proposal operations
produce reviewable artifacts. Commit side effects require the configured
approval policy and an independently recorded receipt.

## Channel Boundary

Gmail ingest is not treated as an ordinary duplex chat transport. Gateway
registers it as a non-duplex event capability; the boundary is documented in
`crates/gateway/src/registry.rs:29-35` and `:115-124`.

A received message may create an input event or goal proposal. It cannot by
itself grant permission, activate a destructive goal or execute an outbound
side effect.

## Data Placement

| Data | Owner |
|---|---|
| provider cursor and delivery dedup | integration/gateway store |
| OAuth credential | credential store |
| normalized durable event | Executive event authority |
| active hypothesis/task context | Agora |
| retained experience/knowledge | Mnemosyne |
| generated file or draft | artifact store with provenance |

Copy only the minimum provider data required by retention policy. Every copied
record keeps account, provider ID, source time and ingest time.

## Sync Invariants

- provider cursors advance only with durable local settlement;
- retries are idempotent;
- deletion/tombstone semantics are explicit;
- account ambiguity asks for selection instead of guessing;
- partial scope is reported as unavailable capability;
- provider errors never produce fabricated success.

## Current Limitations

The presence of adapters and sync modules does not prove every Google write path
is production-ready. Each write capability needs a real provider test,
approval test, retry/idempotency test and durable receipt before it is enabled by
default.
