# Channel and Mobile Communication Architecture

> **Status:** Current channel boundary with future adapters
>
> **Rewritten:** 2026-07-19

## Channel Role

A channel authenticates, normalizes, deduplicates and delivers external input.
It does not own Agent cognition, goals, approvals or provider policy.

Gateway's current boundary is explicit at `crates/gateway/src/lib.rs:1-18`: it
owns provider-neutral intents/effects/store/dispatcher plus Telegram, while
Executive implements the orchestration ports.

```text
Telegram / Gmail / future Web
          |
          v
Gateway transport + durable inbox/outbox
          |
          v
normalized Intent
          |
          v
Executive Turn / Goal / Approval use case
          |
          v
Effect -> Gateway -> provider
```

## Telegram

Telegram long polling and DTO conversion are implemented in
`crates/gateway/src/telegram/mod.rs:1-31`. Required invariants:

- bind external sender/chat to an Aletheon principal;
- persist update cursor and inbox state;
- deduplicate provider retries;
- keep outbound delivery retryable;
- never expose provider token or internal Prompt context;
- route approvals through Executive's approval port.

## Gmail

Gmail is primarily asynchronous event ingest and formal output. Inbound mail is
an untrusted event source. It may propose a goal or deliver information but
cannot activate privileged work without policy and approval.

Outbound mail uses draft/review/commit semantics. Sending is a side effect, not
an automatic consequence of generating text.

## Future Web or Mobile UI

A future web/PWA client is another Gateway/Interact adapter. It must reuse the
same Turn, Goal and Approval use cases rather than introducing a second Agent
loop or state authority.

## Approval Semantics

Approval binds:

```text
principal
operation or goal
exact proposed effect
scope and risk
expiry
single-use decision
```

A channel message saying “yes” is accepted only when it resolves to one live,
owned approval. Ambiguous, expired or wrong-channel decisions fail closed.

## Attachments

Attachments become bounded artifacts with:

- provider and message provenance;
- content type and size limit;
- integrity hash;
- malware/content-policy result where applicable;
- retention and deletion policy.

Raw attachment bytes are not inserted into Prompt history or Agora by default.

## Reliability Acceptance

- duplicate inbound delivery produces one settled action;
- restart resumes cursor/outbox without message loss;
- outbound retry does not duplicate committed side effects;
- principal binding is enforced;
- approval expiry and wrong-owner rejection are tested;
- disabled transports report unavailable state.
