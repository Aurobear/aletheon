# Aletheon Personal Agent Deployment & Integration Plan

> **Status:** Proposed  
> **Scope:** Home deployment, Google ecosystem, mobile communication, Goal Runtime  
> **Principle:** Native Cognit remains primary; external systems are adapters, capabilities, channels, or supervised subagents.

## Document Index

| File | Purpose |
|---|---|
| `01_HOME_DEPLOYMENT_ARCHITECTURE.md` | Linux home server, remote access, service deployment |
| `02_GOOGLE_ECOSYSTEM_INTEGRATION.md` | OAuth, Gmail, Calendar, Drive and synchronization |
| `03_CHANNEL_AND_MOBILE_COMMUNICATION.md` | Telegram, Gmail, Web/PWA and Channel abstraction |
| `04_GOAL_RUNTIME_ARCHITECTURE.md` | `/goal`, iterative execution, retry, verification and model routing |
| `05_IMPLEMENTATION_ROADMAP.md` | Phases, crate layout, milestones and acceptance criteria |
| `06_ALETHEON_NAMING_AND_SYSTEM_IDENTITY.md` | Meaning and formal definition of Aletheon |

## Overall Architecture

```text
                       Google Ecosystem
         Gmail / Calendar / Drive / Contacts / Tasks
                              │
                         OAuth 2.0
                              │
                              ▼
┌──────────────────────────────────────────────────────────┐
│                    Aletheon Server                       │
│                                                          │
│  Dasein                                                  │
│  Native Cognit                                           │
│  Agora                                                   │
│  Goal Runtime                                            │
│  Executive                                               │
│  Mnemosyne ─────────────── GBrain Backend                │
│                                                          │
│  Subagents                                               │
│  ├── DeepSeek Worker                                     │
│  ├── Pi Coding Subagent                                  │
│  └── Future External Agents                              │
│                                                          │
│  Integration Layer                                       │
│  ├── Google Identity                                     │
│  ├── Google Sync Manager                                 │
│  ├── Gmail Capability                                    │
│  ├── Calendar Capability                                 │
│  └── Drive Capability                                    │
│                                                          │
│  Channel Layer                                           │
│  ├── Telegram                                            │
│  ├── Gmail                                               │
│  ├── CLI / TUI                                           │
│  └── Web / PWA                                           │
└──────────────────────────────────────────────────────────┘
                              │
                  Telegram / Gmail / Browser
                              │
                              ▼
                         Mobile User
```

## Strategic Decisions

1. A dedicated mobile app is not required for the first version.
2. Telegram is the primary real-time mobile channel.
3. Gmail is an information source, asynchronous task entry, and formal report channel.
4. Web/PWA is reserved for complex views such as diffs, Goal DAGs, logs, and memory inspection.
5. Aletheon runs continuously on a Linux mini PC or another Linux host.
6. Native Cognit remains the main Agent.
7. DeepSeek is the low-cost iterative worker.
8. Pi is a specialized coding subagent.
9. GPT or Opus handles planning, architecture, escalation, and review.
10. GBrain is a Mnemosyne backend, not the complete memory system.
11. Google integrations are independent capabilities governed by Executive.
12. Channel-specific logic must not enter the core Agent architecture.

## First Usable Loop

```text
Telegram /goal
    ↓
Native Cognit compiles intent
    ↓
Goal Runtime creates and advances a plan
    ↓
DeepSeek or Pi executes bounded work
    ↓
Executive captures outputs and enforces limits
    ↓
Verifier checks tests, diff and policy
    ↓
Telegram sends progress or approval request
    ↓
Goal completes
    ↓
Mnemosyne/GBrain records outcome and lessons
```
