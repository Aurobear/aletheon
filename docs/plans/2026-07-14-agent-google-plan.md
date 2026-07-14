# Agent-Google Integration — Implementation Plan

> **For agentic workers:** Use `/workflow feature` to implement this plan phase-by-phase. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the 8-phase Aletheon agent-Google integration: Telegram channel, Goal Runtime, DeepSeek/Pi workers, verification pipeline, Google OAuth+sync, Gmail channel+approval, and GBrain backend.

**Architecture:** Integrate into existing crates (executive, corpus, mnemosyne, fabric). Extend existing `ObjectiveStore`/`AgentRuntime`/`ModelRouter` rather than greenfield. Channel abstraction layer routes external messages through existing `DaemonTurnOrchestrator`.

**Tech Stack:** Rust (tokio async), rusqlite (SQLite), reqwest (HTTP), teloxide (Telegram), AES-256-GCM (credential vault), Docker Compose (GBrain+PostgreSQL).

**Spec:** `docs/plans/2026-07-14-agent-google-design.md`

**Phase dependency graph:**
```
A ──→ B ──→ C ──→ D ──→ E
              │
              └──→ F ──→ G
                        │
              └──→ H ──┘
```
---
