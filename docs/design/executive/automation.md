# Automation / Routines System

> New document — code paths updated to match actual crate names (fabric, cognit, corpus, dasein, mnemosyne, metacog, interact, executive)

> P3 automation system providing cron-triggered, webhook-triggered, and API-triggered automations with multi-channel delivery, script pre-processing, and daily-run limits.

**Crate:** `executive`
**Code location:** `runtime/src/impl/automation/`
**Last Updated:** 2026-06-14

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| AutomationScheduler | Implemented | `runtime/src/impl/automation/mod.rs` | Top-level scheduler with CRUD, cron/webhook triggers |
| CronParser | Implemented | `runtime/src/impl/automation/cron.rs` | Cron expression parsing and matching |
| DeliveryManager | Implemented | `runtime/src/impl/automation/delivery.rs` | Multi-channel delivery (Telegram, Discord, Slack, Email, Webhook, Local, Stdout) |
| ScriptRunner | Implemented | `runtime/src/impl/automation/script.rs` | Script pre-processing execution |
| WebhookEvent | Implemented | `runtime/src/impl/automation/webhook.rs` | Webhook event matching and HMAC verification |

---

## 1. Overview

The automation system enables scheduled and event-driven agent workflows. Automations can be triggered by cron schedules, webhook events, or API calls. Agent responses are delivered to configured channels (Telegram, Discord, Slack, email, webhook, local file, stdout).

---

## 2. Automation Model

```rust
struct Automation {
    id: String,
    name: String,
    trigger: AutomationTrigger,
    prompt: String,
    script: Option<PathBuf>,        // Optional pre-processing script
    skills: Vec<String>,
    delivery: Vec<DeliveryTarget>,
    model: Option<String>,
    daily_limit: u32,
    daily_count: u32,
    last_run: Option<u64>,
}

enum AutomationTrigger {
    Cron { expression: String },
    Webhook { events: Vec<String>, hmac_secret: String },
    Api { endpoint: String },
}

enum DeliveryTarget {
    Telegram { chat_id: Option<String> },
    Discord { channel_id: Option<String> },
    Slack { channel: Option<String> },
    Email { address: String },
    Webhook { url: String },
    Local { path: PathBuf },
    Stdout,
}
```

---

## 3. Trigger Types

### 3.1 Cron Trigger

Standard cron expression parsing and matching. The scheduler evaluates all cron-triggered automations against the current time, respecting daily limits.

Code location: `runtime/src/impl/automation/cron.rs`

### 3.2 Webhook Trigger

Matches incoming webhook events against configured event types. HMAC signature verification is expected to be done by the caller before passing to the scheduler.

Code location: `runtime/src/impl/automation/webhook.rs`

### 3.3 API Trigger

Endpoint-based trigger (designed, integration with daemon HTTP server needed).

---

## 4. Execution Flow

1. **Trigger fires** (cron match, webhook event, API call)
2. **Daily limit check** — skip if daily_count >= daily_limit
3. **Optional script pre-processing** — if `script` is set, execute via `ScriptRunner`
4. **Agent execution** — in a full system this would invoke the LLM; the module returns prompt as-is for testability
5. **SILENT check** — if output equals `"[SILENT]"`, skip delivery
6. **Multi-channel delivery** — deliver to all configured targets via `DeliveryManager`
7. **Counter update** — increment daily_count, update last_run

Code location: `runtime/src/impl/automation/mod.rs`

---

## 5. Delivery System

`DeliveryManager` handles multi-channel delivery. Each `DeliveryTarget` variant has its own delivery logic.

**Supported channels:**
- **Telegram** — Bot API with optional chat_id
- **Discord** — Webhook with optional channel_id
- **Slack** — Webhook with optional channel
- **Email** — SMTP delivery
- **Webhook** — HTTP POST with JSON payload
- **Local** — Write to local file path
- **Stdout** — Print to standard output

Code location: `runtime/src/impl/automation/delivery.rs`

---

## 6. Daily Limits

Each automation has a `daily_limit` and `daily_count`. The count resets at midnight via `reset_daily_counts()`. The limit is checked both at trigger time and execution time (double-check).

---

## 7. Silent Marker

If the agent's response equals `"[SILENT]"`, the delivery step is skipped entirely. This allows automations to conditionally suppress output.

---

## 8. Design Notes

- **Self-contained testing:** The module is designed to be testable without an LLM backend; `execute_automation()` takes `agent_output` as a parameter
- **Thread safety:** `AutomationScheduler` is not currently async-safe; concurrent access would need external synchronization
- **Future work:** API trigger integration with daemon HTTP server, LLM invocation integration, persistent automation storage

---

## Implementation Summary

| Component | Code Location | Key Types |
|-----------|---------------|-----------|
| AutomationScheduler | `runtime/src/impl/automation/mod.rs` | `AutomationScheduler`, `Automation`, `AutomationTrigger`, `DeliveryTarget`, `AutomationResult` |
| CronParser | `runtime/src/impl/automation/cron.rs` | `CronParser`, `CronSchedule` |
| DeliveryManager | `runtime/src/impl/automation/delivery.rs` | `DeliveryManager` |
| ScriptRunner | `runtime/src/impl/automation/script.rs` | `ScriptRunner` |
| WebhookEvent | `runtime/src/impl/automation/webhook.rs` | `WebhookEvent`, `matches_event_type()` |

**Test coverage:** 14+ tests covering CRUD operations, cron matching, daily limits, webhook triggers, execution with delivery, SILENT marker, counter increments, scheduler lifecycle.
