# E7 channel contract owner cutover — 2026-08-12

## Requirement anchors

- E7 uniquely removes extension-specific Fabric rich rows and only their root
  re-exports: `docs/plans/2026-08-08-preserved-extensions-cutover.md:418-428`.
- D6 must run after E7 and owns non-extension/shared-root closeout:
  `docs/plans/2026-08-08-domain-authority-and-adapter-extraction.md:334-343`.

## Before / after

```text
Before: Gateway / Telegram / SQLite / daemon -> fabric::channel rich DTOs
After:  Gateway / Telegram / SQLite / daemon -> gateway::channel DTOs
                                             (one owner, no Fabric alias)
```

The neutral channel DTOs now live at `crates/gateway/src/channel.rs:1-104` and
are exported by `crates/gateway/src/lib.rs:10-18`. All Rust callers use the
Gateway owner path. `crates/gateway/src/channel/mod.rs`, its `types/mod.rs`
declaration, and both Fabric root re-export blocks were removed. The unrelated
IPC V2 UUID `MessageId` remains the ownerless contract exported by
`crates/fabric/src/contracts.rs:19`; it is not the removed channel string ID.

## Caller-zero gate

```bash
! rg -n 'fabric::(types::)?channel|fabric::(ActionType|ChannelHealth|ChannelId|ConversationId|ExternalSenderId|InboundMessage|MessageContent|OutboundMessage|UserAction)' crates --glob '*.rs'
```

## Focused validation

- `bash scripts/cargo-agent.sh check -p gateway -p gateway-channel-telegram -p adapters-sqlite -p executive -p aletheon --all-targets`: PASS.
- Gateway: 9 passed.
- Telegram adapter: 25 passed.
- SQLite channel tests: 6 passed.
- Executive channel/approval/Telegram/Google focused suites: 23 passed.

Changed validation and installed-runtime acceptance follow this evidence update.
