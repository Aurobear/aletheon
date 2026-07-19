# Aletheon Personal Agent Architecture

> **Status:** Product architecture index
>
> **Rewritten:** 2026-07-19

## Purpose

This document set describes Aletheon as a persistent personal Agent across
home deployment, communication channels, Google capabilities and durable goals.
It does not define new crates or an implementation roadmap.

## Document Index

| Document | Scope |
|---|---|
| `01_HOME_DEPLOYMENT_ARCHITECTURE.md` | single-instance deployment, storage, supervision and backup |
| `02_GOOGLE_ECOSYSTEM_INTEGRATION.md` | Google identity, read/sync capabilities and data boundaries |
| `03_CHANNEL_AND_MOBILE_COMMUNICATION.md` | Telegram, Gmail and future web channel semantics |
| `04_GOAL_RUNTIME_ARCHITECTURE.md` | persistent goal lifecycle, attempts, verification and recovery |
| `06_ALETHEON_NAMING_AND_SYSTEM_IDENTITY.md` | accepted system identity and crate naming policy |

## Product Loop

```text
User or environment
    -> Gateway channel
    -> Executive Operation / Goal
    -> Cognit or selected Runtime
    -> governed Corpus capability
    -> evidence and independent verification
    -> Mnemosyne / Agora / durable event
    -> reply or continued supervision
```

Current implementation evidence:

- The application root exposes daemon, exec, config and doctor modes at
  `crates/aletheon/src/main.rs:1-23`.
- Gateway owns provider-neutral channel dispatch plus Telegram at
  `crates/gateway/src/lib.rs:1-18`.
- Executive exposes typed Goal use cases at
  `crates/executive/src/service/goal_service.rs:43-72`.
- Google adapters live under `crates/corpus/src/tools/google/` and are registered
  by Executive only when account scope permits.

## Authority Boundaries

- Gateway authenticates and normalizes external input; it does not own goals.
- Executive owns orchestration, approval, global verification and settlement.
- Kernel owns lifecycle and admission mechanisms.
- Runtime executes assigned work but does not grant permission or self-verify.
- Corpus owns tool/provider execution, including Google and MCP adapters.
- Mnemosyne owns memory policy; external stores are providers, not authorities.

## Delivery Rule

A feature is not complete because its type, adapter or configuration exists. It
is complete only when the real entry path, authority checks, durable recovery
and end-to-end acceptance are all demonstrated.
