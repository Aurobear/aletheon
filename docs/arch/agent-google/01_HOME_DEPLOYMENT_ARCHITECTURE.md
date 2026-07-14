# Home Deployment Architecture

> **Status:** Proposed  
> **Target:** One-user continuously running Aletheon deployment

## 1. Deployment Decision

For a dedicated Aletheon server, prefer a Linux mini PC over buying a Mac specifically for deployment.

Reasons:

- Aletheon is Rust and Linux oriented.
- Executive will benefit from namespaces, Landlock, seccomp, bubblewrap and native process control.
- Pi, GBrain, containers, worktrees and robotics tooling fit Linux naturally.
- The machine may later connect to Linux-based robot systems.
- Linux mini PCs provide better RAM and storage flexibility for the budget.

A Mac mini is reasonable only when it will also be used as a desktop, Apple ecosystem machine, or local model host.

## 2. Hardware

### Minimum

```text
CPU: Intel N100/N150 or similar
RAM: 16 GB
SSD: 512 GB
Network: Gigabit Ethernet
```

### Recommended

```text
CPU: AMD 7840HS / 8845HS or equivalent
RAM: 32 GB
SSD: 1 TB NVMe
Network: Gigabit or 2.5 GbE
```

This supports Aletheon, GBrain, PostgreSQL/PGLite, Pi subagents, multiple Goal workers, local embeddings and future robot integration.

Use 64 GB RAM only when planning local LLM inference, heavy document ingestion or many concurrent agents.

## 3. Operating System

Recommended:

```text
Ubuntu Server LTS
```

Alternative:

```text
Debian Stable
```

The server should support unattended boot, automatic restart, health checks, encrypted secrets and backups.

## 4. Service Layout

```text
Host Linux
├── aletheon-core
├── aletheon-telegram
├── aletheon-google-sync
├── gbrain
├── postgres or pglite
├── reverse proxy
└── backup jobs
```

Recommended split:

```text
Native host process:
- Aletheon Executive
- Native Cognit
- process supervision
- Pi process control
- sandbox and worktree management

Containers:
- GBrain
- PostgreSQL
- optional object storage
- optional reverse proxy
```

## 5. Remote Access

For private use, use Tailscale or another WireGuard mesh.

```text
Phone
   ↓ encrypted private network
Tailscale
   ↓
Home Linux Server
```

Telegram long polling needs no inbound public connection.

Public exposure should only be added after authentication, device sessions, rate limiting, audit logs, CSRF protection, webhook verification and secret rotation exist.

## 6. Supervision

System-level services:

```text
systemd
├── aletheon.service
├── docker.service
└── tailscaled.service
```

Internal Agent processes remain managed by Executive:

```text
Aletheon Executive
├── Native Agent Process
├── DeepSeek Worker
├── Pi Subagent
├── Goal Supervisor
├── Mnemosyne Consolidator
└── Google Sync Workers
```

Do not turn every worker into a systemd service.

## 7. Storage

```text
/var/lib/aletheon/
├── state/
├── goals/
├── sessions/
├── mnemosyne/
├── gbrain/
├── artifacts/
├── worktrees/
└── audit/

/etc/aletheon/
├── config.toml
└── policy/

/run/aletheon/
└── sockets/
```

Secrets must stay outside Git and model context.

## 8. Backup

Back up:

- Dasein snapshots;
- Goal database;
- Mnemosyne metadata;
- GBrain database;
- Google sync cursors;
- configuration and encrypted credentials;
- user artifacts and architecture decisions.

Suggested policy:

```text
Daily local snapshot
Weekly encrypted remote backup
Monthly restore test
```

## 9. Acceptance Criteria

The deployment is ready when:

- it boots without a monitor;
- Aletheon restarts automatically;
- Telegram reaches Aletheon;
- Goals continue after the phone disconnects;
- Goal state survives restart;
- worker crashes do not crash the system;
- secrets are absent from logs and Git;
- backups can be restored;
- administration works through Tailscale.
